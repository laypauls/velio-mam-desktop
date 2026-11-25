//! 모델·종류별 flat 인덱스 세대 (P3-5) — 구축·교체·증분 추가·재구축·검색 후보.
//!
//! - 세대 디렉터리: `<key_dir>/gen/g<번호>`, 활성 포인터 `ACTIVE` ([`IndexRoot`] 재사용). 인덱스는 f16 flat.
//! - 인덱스는 파생 데이터다: [`Generations::rebuild`] 는 SQLite 활성 행 + raw 파일만 읽는다 (db.md §1).
//! - 삭제·재임베딩은 인덱스에서 지우지 않는다. 검색이 SQLite 활성 집합을 `allowed` 로 넘겨 거른다 (flat tombstone 불필요).
//! - 증분 추가는 "SQLite 에는 있는데 인덱스에는 없는 활성 id" 를 raw 에서 읽어 더한다 — 상태가 없어 재시작 후에도 이어진다.
//! - Windows: view 핸들이 살아 있으면 옛 세대 디렉터리 삭제가 실패한다 — 정리는 최선 노력이며 다음 재구축에서 다시 시도한다.

use std::path::PathBuf;

use rusqlite::Connection;

use crate::active_cache::ActiveMapCache;
use crate::embed_store::{EmbedStore, ModelKey, ReadReport};
use crate::error::{Result, VectorError};
use crate::flat::FlatIndex;
use crate::fusion::{CandidatePlan, vector_candidates, vector_candidates_with_set};
use crate::group::AssetHit;
use crate::meta;
use crate::store::{Engine, IndexSpec, save_index};
use crate::store_version::save_index_version;
use crate::swap::IndexRoot;
use crate::types::{AnnIndex, EmbeddingId, OpenMode, Quantization};

/// 제품 인덱스 설정: flat, f16 (P1-3 결론), 코사인.
pub fn index_spec(key: &ModelKey) -> IndexSpec {
    IndexSpec {
        engine: Engine::Flat,
        model_id: key.model_id.clone(),
        model_version: key.model_version.clone(),
        dimension: key.dimension,
        quantization: Quantization::F16,
    }
}

/// 재구축 결과.
#[derive(Debug, PartialEq, Eq)]
pub struct RebuildReport {
    pub generation: String,
    pub vectors: u64,
    /// raw 파일에서 읽을 수 없어 빠진 레코드
    pub skipped: Vec<EmbeddingId>,
}

pub struct Generations {
    dir: PathBuf,
    root: IndexRoot,
}

impl Generations {
    pub fn new(store: &EmbedStore, key: &ModelKey) -> Result<Self> {
        let dir = store.key_dir(key).join("gen");
        Ok(Self {
            root: IndexRoot::new(&dir)?,
            dir,
        })
    }

    pub fn active_name(&self) -> Result<Option<String>> {
        self.root.active()
    }

    fn next_name(&self) -> Result<String> {
        let mut max = 0u64;
        for e in std::fs::read_dir(&self.dir)? {
            let n = e?.file_name();
            if let Some(num) = n.to_str().and_then(|s| s.strip_prefix('g')) {
                max = max.max(num.parse().unwrap_or(0));
            }
        }
        Ok(format!("g{}", max + 1))
    }

    /// 새 세대를 SQLite+raw 에서 구축해 활성화하고 옛 세대를 정리한다. 구축이 실패하면 활성 포인터는 그대로다.
    pub fn rebuild(
        &self,
        conn: &Connection,
        store: &EmbedStore,
        key: &ModelKey,
    ) -> Result<RebuildReport> {
        let name = self.next_name()?;
        let spec = index_spec(key);
        let hint = store.asset_map(conn, key)?.len();
        let mut report = ReadReport::default();
        self.root.build_generation(&name, |dir| {
            let idx = FlatIndex::with_capacity(key.dimension, spec.quantization, hint);
            report = store.for_each_active(conn, key, |_| true, |id, v| idx.add(id, v))?;
            save_index(dir, &idx, &spec)
        })?;
        // 활성화 실패 시 방금 만든 세대를 지워 시도마다 고아 디렉터리가 쌓이지 않게 한다 (포인터는 그대로).
        if let Err(e) = self.root.activate(&name, &spec) {
            let _ = std::fs::remove_dir_all(self.root.dir(&name));
            return Err(e);
        }
        self.cleanup(&name);
        Ok(RebuildReport {
            generation: name,
            vectors: report.read,
            skipped: report.skipped,
        })
    }

    /// 활성이 아닌 세대 디렉터리를 지운다 (실패는 무시 — view 핸들이 남은 Windows).
    fn cleanup(&self, active: &str) {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str()
                && n != active
                && n.starts_with('g')
                && e.path().is_dir()
            {
                let _ = self.root.retire(n);
            }
        }
    }

    /// 활성 세대를 연다. 활성 세대가 없으면 `None`. 설정(모델·버전·차원)이 다르면 `IndexMismatch`, 손상이면 `Corrupt`.
    pub fn open_active(&self, key: &ModelKey, mode: OpenMode) -> Result<Option<FlatIndex>> {
        let Some(name) = self.root.active()? else {
            return Ok(None);
        };
        let spec = index_spec(key);
        let dir = self.root.dir(&name);
        if !meta::check(&dir, &spec)? {
            return Err(VectorError::IndexMismatch(format!(
                "active generation '{name}' has no meta"
            )));
        }
        crate::store_version::open_current(&dir, &spec, mode).map(Some)
    }

    /// 열 수 없거나(없음·손상·설정 불일치) 활성 세대가 없으면 재구축한 뒤 연다. 재구축했으면 보고를 함께 돌려준다.
    pub fn open_or_rebuild(
        &self,
        conn: &Connection,
        store: &EmbedStore,
        key: &ModelKey,
        mode: OpenMode,
    ) -> Result<(FlatIndex, Option<RebuildReport>)> {
        match self.open_active(key, mode) {
            Ok(Some(idx)) => return Ok((idx, None)),
            Ok(None)
            | Err(VectorError::IndexMismatch(_) | VectorError::Corrupt(_) | VectorError::Io(_)) => {
            }
            Err(e) => return Err(e),
        }
        let report = self.rebuild(conn, store, key)?;
        let idx = self
            .open_active(key, mode)?
            .ok_or_else(|| VectorError::Corrupt("generation vanished after rebuild".into()))?;
        Ok((idx, Some(report)))
    }

    /// 메모리 인덱스를 활성 세대에 저장한다 (Load 모드로 연 인덱스). 저장 전에 죽어도 재시작 시 증분 동기화가 메운다.
    ///
    /// **열려 있을 수 있는 파일을 바꾸지 않는다**: 같은 세대 디렉터리에 새 데이터 버전(`index.<n>.flat`)을 완성한 뒤
    /// `DATA` 포인터만 원자적으로 바꾼다 (메타·세대 이름·`ACTIVE` 는 그대로 — [`ActiveMapCache`] 재로드 없음).
    /// 읽는 쪽은 이미 연 버전을 계속 쓰고 다음 `open_active` 에서 새 버전을 본다. 비용은 인덱스 전체 쓰기 1회다.
    pub fn persist(&self, key: &ModelKey, index: &FlatIndex) -> Result<()> {
        let name = self
            .root
            .active()?
            .ok_or_else(|| VectorError::InvalidInput("no active generation".into()))?;
        save_index_version(&self.root.dir(&name), index, &index_spec(key))?;
        self.cleanup_older(&name);
        Ok(())
    }

    /// [`Self::persist`] 와 같되, 현재 데이터 파일이 `min_age` 보다 젊으면 저장하지 않는다 (`Ok(false)`).
    /// 저장하지 않아도 재시작 시 `sync_incremental` 이 메우므로 안전하다 — 대용량에서 틱마다 전체 쓰기를 피한다.
    pub fn persist_if_older(
        &self,
        key: &ModelKey,
        index: &FlatIndex,
        min_age: std::time::Duration,
    ) -> Result<bool> {
        let name = self
            .root
            .active()?
            .ok_or_else(|| VectorError::InvalidInput("no active generation".into()))?;
        let dir = self.root.dir(&name);
        if !crate::store_version::is_older_than(&dir, Engine::Flat, min_age) {
            return Ok(false);
        }
        self.persist(key, index)?;
        Ok(true)
    }

    /// 활성보다 번호가 작은 세대 디렉터리만 지운다 (이전 재구축에서 매핑 때문에 못 지운 것의 재시도).
    /// 번호가 큰 디렉터리는 다른 쪽이 구축 중일 수 있어 건드리지 않는다.
    fn cleanup_older(&self, active: &str) {
        let num = |s: &str| s.strip_prefix('g').and_then(|n| n.parse::<u64>().ok());
        let Some(active_n) = num(active) else { return };
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in rd.flatten() {
            if let Some(n) = e.file_name().to_str()
                && num(n).is_some_and(|x| x < active_n)
                && e.path().is_dir()
            {
                let _ = self.root.retire(n);
            }
        }
    }
}

/// SQLite 활성 집합에는 있고 인덱스에는 없는 벡터를 raw 에서 읽어 더한다. 추가 건수와 건너뛴 id 를 돌려준다.
/// 인덱스는 Load 모드여야 한다 (view 는 `ReadOnly`).
pub fn sync_incremental(
    conn: &Connection,
    store: &EmbedStore,
    key: &ModelKey,
    index: &FlatIndex,
) -> Result<ReadReport> {
    store.for_each_active(
        conn,
        key,
        |id| !index.contains(id),
        |id, v| index.add(id, v),
    )
}

/// 자산 단위 벡터 후보 — **활성 맵 캐시판** (앱 경로는 이쪽). SQLite 활성 집합(폐기된 벡터 제외)과
/// 호출자 필터 `allowed`(임베딩 id) 의 교집합 안에서만 찾는다. 맵은 `cache` 의 스냅샷을 쓰므로 질의마다
/// 활성 행을 읽지 않는다 ([`ActiveMapCache`] 의 무효화 규칙 참고). `allowed = None` 이면 활성 전체.
/// 자산 id 로 허용 집합을 만들려면 `cache.get_or_load(..)?.embeddings_of_assets(..)` 를 쓴다.
#[allow(clippy::too_many_arguments)]
pub fn active_candidates_cached(
    conn: &Connection,
    store: &EmbedStore,
    cache: &ActiveMapCache,
    key: &ModelKey,
    index: &FlatIndex,
    query: &[f32],
    plan: &CandidatePlan,
    allowed: Option<&[EmbeddingId]>,
) -> Result<Vec<AssetHit>> {
    let map = cache.get_or_load(conn, store, key)?;
    let asset_of = |id: EmbeddingId| map.asset_of(id);
    match allowed {
        Some(a) => {
            let ids: Vec<EmbeddingId> = a.iter().copied().filter(|&id| map.contains(id)).collect();
            vector_candidates_with_set(index, query, plan, Some(&ids), None, index.len(), &asset_of)
        }
        None => vector_candidates_with_set(
            index,
            query,
            plan,
            Some(map.ids()),
            Some(map.present()),
            index.len(),
            &asset_of,
        ),
    }
}

/// 자산 단위 벡터 후보 (캐시 없음). SQLite 활성 집합(폐기된 벡터 제외)과 호출자 필터 `allowed` 의 교집합 안에서만 찾는다.
///
/// **느리다**: 호출마다 `EmbedStore::asset_map` 으로 활성 행 전체를 읽는다 (220만 행 1.3~1.7초, P4 측정).
/// 호환용으로만 남겼다 — 앱 경로는 [`active_candidates_cached`] 를 쓴다.
pub fn active_candidates(
    conn: &Connection,
    store: &EmbedStore,
    key: &ModelKey,
    index: &FlatIndex,
    query: &[f32],
    plan: &CandidatePlan,
    allowed: Option<&[EmbeddingId]>,
) -> Result<Vec<AssetHit>> {
    let map = store.asset_map(conn, key)?;
    let ids: Vec<EmbeddingId> = match allowed {
        Some(a) => a
            .iter()
            .copied()
            .filter(|id| map.contains_key(id))
            .collect(),
        None => map.keys().copied().collect(),
    };
    vector_candidates(index, query, plan, Some(&ids), index.len(), &|id| {
        map.get(&id).copied()
    })
}
