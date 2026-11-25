//! 제품 원시 임베딩 저장 (P3-5) — f16 추가 전용 파일 + SQLite `embeddings` 메타.
//!
//! - 키 `embedding_id` 는 AUTOINCREMENT (재사용 금지). 자산·구간·종류·모델 ID/버전·차원을 함께 기록한다 (db.md §2).
//! - 쓰기 순서: raw 파일 append + fsync -> SQLite 한 트랜잭션(옛 행 superseded + 새 행). 중간에 죽으면 파일 꼬리에
//!   고아 레코드만 남고, SQLite 가 가리키는 레코드가 없을 수는 없다. 파일은 추가 전용이라 재임베딩한 옛 레코드는 오프셋만 버린다.
//! - 인덱스는 이 저장소(SQLite 활성 행 + raw 파일)만으로 다시 만들 수 있다 ([`crate::generation`]).
//!
//! 레이아웃: `<root>/<kind>/<model_key>/raw.f16`.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use memmap2::Mmap;
use rusqlite::{Connection, params};

use crate::embed_state::{self, ChangeEpoch, Floors};
pub use crate::embed_types::{ModelKey, NewVector, ReadReport, VectorKind, VectorSource};
use crate::error::Result;
use crate::raw::{RawFile, f16_bytes_to_f32, is_hole};
use crate::sql;
use crate::types::{EmbeddingId, normalized, validate_vector};

/// 원시 임베딩 저장소. 프로세스 안에서 쓰기는 직렬화한다 (쓰기 프로세스는 하나라고 가정한다).
pub struct EmbedStore {
    root: PathBuf,
    write_lock: Mutex<()>,
    /// 키별 raw 참조 하한 캐시 — `put` 이 행 수에 비례하는 `MAX(raw_offset)` 를 매번 돌지 않게 한다.
    floors: Floors,
}

impl EmbedStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
            floors: Floors::default(),
        })
    }

    /// 이 키의 임베딩이 `put` 으로 바뀔 때마다 달라지는 프로세스 전역 표식 ([`crate::ActiveMapCache`] 가 사용).
    pub fn change_epoch(&self, key: &ModelKey) -> ChangeEpoch {
        embed_state::epoch(&self.key_dir(key))
    }

    /// `put` 을 거치지 않고 `embeddings` 를 바꾼 경우(자산 영구 삭제·초기화·직접 SQL) 호출해
    /// 활성 맵 캐시가 다음 질의에서 다시 확인하게 한다.
    pub fn notify_changed(&self, key: &ModelKey) {
        embed_state::bump_reset(&self.key_dir(key));
    }

    /// SQLite 가 참조하는 가장 끝 raw 오프셋(+레코드 길이). 키의 **모든 행**을 훑으므로(220만 행 약 0.6초)
    /// 키·프로세스당 처음 한 번과 손상 의심 때만 부른다.
    fn referenced_end(conn: &Connection, key: &ModelKey) -> Result<u64> {
        let max: Option<i64> = conn.query_row(
            crate::sql::RAW_FLOOR,
            params![
                key.kind.as_str(),
                key.model_id,
                key.model_version,
                key.dimension as i64
            ],
            |r| r.get(0),
        )?;
        Ok(max
            .map(|o| (o.max(0) as u64).saturating_add(RawFile::record_len(key.dimension)))
            .unwrap_or(0))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 종류·모델별 디렉터리 (raw 파일과 인덱스 세대가 들어간다).
    pub fn key_dir(&self, key: &ModelKey) -> PathBuf {
        self.root.join(key.kind.as_str()).join(key.dir_name())
    }

    pub fn raw_path(&self, key: &ModelKey) -> PathBuf {
        self.key_dir(key).join("raw.f16")
    }

    /// 자산의 (종류, 모델) 벡터를 `vectors` 로 교체한다: 기존 활성 행은 `superseded_at` 을 채우고 새 행을 넣는다.
    /// `vectors` 가 비면 기존 벡터만 폐기한다. 새 `embedding_id` 들을 입력 순서대로 돌려준다.
    /// 검증 실패(차원·NaN·영벡터)면 파일·DB 모두 건드리지 않는다.
    pub fn put(
        &self,
        conn: &mut Connection,
        key: &ModelKey,
        asset_id: i64,
        vectors: &[NewVector<'_>],
        now: i64,
    ) -> Result<Vec<EmbeddingId>> {
        for v in vectors {
            validate_vector(key.dimension, v.vector)?;
        }
        // 삭제 표시된 자산(폴더 제거 뒤 늦게 끝난 임베딩 작업)에는 벡터를 쓰지 않는다 - raw 파일에 덧붙이기 전에 먼저 거른다
        if Self::asset_inactive(conn, asset_id)? {
            return Ok(Vec::new());
        }
        let _guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let offsets = if vectors.is_empty() {
            Vec::new()
        } else {
            std::fs::create_dir_all(self.key_dir(key))?;
            let norm: Vec<Vec<f32>> = vectors.iter().map(|v| normalized(v.vector)).collect();
            let refs: Vec<&[f32]> = norm.iter().map(Vec::as_slice).collect();
            // SQLite 가 (폐기 행 포함) 이미 가리킨 영역은 파일이 잘렸어도 재사용하지 않는다 — 손상된 행이
            // 새 레코드와 오프셋이 겹쳐 다른 자산의 벡터를 읽는 것을 막는다 (RawFile::open_with_floor 참고).
            // floor(= 참조 끝)는 메모리에 기억해 두고 갱신한다. 파일이 기억한 floor 보다 짧으면(꼬리 손상·초기화)
            // 기억을 믿지 않고 DB 로 다시 구한다 — 이때만 O(n) 이다. 처음 put 도 한 번 구한다.
            let dir = self.key_dir(key);
            let mut raw = RawFile::open(&self.raw_path(key), key.dimension)?;
            let floor = match self.floors.get(&dir) {
                Some(f) if raw.len_bytes() >= f => f,
                _ => Self::referenced_end(conn, key)?,
            };
            raw.ensure_floor(floor)?;
            let offsets = raw.append(&refs)?;
            raw.sync()?; // 파일 -> SQLite 순서
            offsets
        };
        let tx = conn.transaction()?;
        // 같은 트랜잭션 안에서 다시 확인한다 (위 확인과 쓰기 사이에 삭제 표시됐을 수 있다). 덧붙인 raw 바이트는 참조되지 않는다
        if Self::asset_inactive(&tx, asset_id)? {
            return Ok(Vec::new());
        }
        tx.execute(
            sql::SUPERSEDE,
            params![
                now,
                asset_id,
                key.kind.as_str(),
                key.model_id,
                key.model_version
            ],
        )?;
        let mut ids = Vec::with_capacity(vectors.len());
        {
            let mut ins = tx.prepare(
                "INSERT INTO embeddings (asset_id, segment_id, vector_kind, source, frame_index,
                    model_id, model_version, dimension, raw_offset, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for (v, off) in vectors.iter().zip(&offsets) {
                ins.execute(params![
                    asset_id,
                    v.segment_id,
                    key.kind.as_str(),
                    v.source.as_str(),
                    v.frame_index,
                    key.model_id,
                    key.model_version,
                    key.dimension as i64,
                    *off as i64,
                    now
                ])?;
                ids.push(tx.last_insert_rowid() as u64);
            }
        }
        tx.commit()?;
        let dir = self.key_dir(key);
        if let Some(&last) = offsets.last() {
            self.floors
                .set(&dir, last + RawFile::record_len(key.dimension));
        }
        embed_state::record_touch(&dir, asset_id as u64); // 커밋 뒤에 기록한다 — 캐시가 본 순번보다 데이터가 앞서 있을 수는 있어도 뒤처지지 않는다
        Ok(ids)
    }

    /// 자산 행이 있고 `active` 가 아닌가 (행이 없으면 FK 가 막으므로 여기서는 `false`).
    fn asset_inactive(conn: &Connection, asset_id: i64) -> Result<bool> {
        Ok(conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM media_assets WHERE asset_id = ?1 AND status <> 'active')",
            [asset_id],
            |r| r.get(0),
        )?)
    }

    /// 활성(폐기되지 않은) 임베딩 id, 오름차순.
    pub fn active_ids(&self, conn: &Connection, key: &ModelKey) -> Result<Vec<EmbeddingId>> {
        let mut ids: Vec<_> = self.asset_map(conn, key)?.into_keys().collect();
        ids.sort_unstable();
        Ok(ids)
    }

    /// 활성 `embedding_id -> asset_id`.
    pub fn asset_map(
        &self,
        conn: &Connection,
        key: &ModelKey,
    ) -> Result<HashMap<EmbeddingId, u64>> {
        let mut stmt = conn.prepare_cached(
            "SELECT embedding_id, asset_id FROM embeddings
             WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND dimension = ?4
               AND superseded_at IS NULL",
        )?;
        let rows = stmt.query_map(
            params![
                key.kind.as_str(),
                key.model_id,
                key.model_version,
                key.dimension as i64
            ],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// 활성 행을 `embedding_id` 순으로 raw 파일에서 읽어 `f` 에 넘긴다 (`include` 가 거짓이면 건너뜀).
    /// raw 에서 읽을 수 없는 레코드(오프셋이 파일 밖·파일 없음)는 건너뛰고 [`ReadReport::skipped`] 로 보고한다.
    pub fn for_each_active(
        &self,
        conn: &Connection,
        key: &ModelKey,
        include: impl Fn(EmbeddingId) -> bool,
        mut f: impl FnMut(EmbeddingId, &[f32]) -> Result<()>,
    ) -> Result<ReadReport> {
        let map = self.map_raw(key)?;
        let rl = RawFile::record_len(key.dimension) as usize;
        let mut stmt = conn.prepare(crate::sql::ACTIVE_RAW_OFFSETS)?;
        let mut rows = stmt.query(params![
            key.kind.as_str(),
            key.model_id,
            key.model_version,
            key.dimension as i64
        ])?;
        let (mut report, mut buf) = (ReadReport::default(), Vec::with_capacity(key.dimension));
        while let Some(r) = rows.next()? {
            let id = r.get::<_, i64>(0)? as u64;
            if !include(id) {
                continue;
            }
            let off = usize::try_from(r.get::<_, i64>(1)?).unwrap_or(usize::MAX);
            let rec = map.as_ref().and_then(|m| m.get(off..off.checked_add(rl)?));
            // 손상 복구 때 채운 구간(HOLE)은 잃은 레코드다 -> 건너뜀으로 보고
            let rec = rec.filter(|r| !is_hole(r));
            let Some(rec) = rec else {
                report.skipped.push(id);
                continue;
            };
            f16_bytes_to_f32(rec, &mut buf)?;
            f(id, &buf)?;
            report.read += 1;
        }
        Ok(report)
    }

    /// raw 파일의 읽기 전용 매핑 (파일이 없거나 비어 있으면 `None`).
    fn map_raw(&self, key: &ModelKey) -> Result<Option<Mmap>> {
        match File::open(self.raw_path(key)) {
            Ok(file) if file.metadata()?.len() > 0 => {
                // SAFETY: 읽기 전용 매핑. 파일은 추가만 되고 줄어들지 않는다(RawFile::open 의 꼬리 정리는 쓰기 잠금 밖에서
                // 일어나지 않는다고 가정하지 않는다 -> 범위 밖 접근은 `get` 이 None 으로 막는다).
                Ok(Some(unsafe { Mmap::map(&file)? }))
            }
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 한 자산의 활성 벡터 전부(프레임 순서 = `embedding_id` 순). `idx_embeddings_asset` 로 찾으므로 전체 스캔이 없다.
    /// raw 에서 읽을 수 없는 레코드는 건너뛴다. 유사 미디어 질의(`find_similar`)의 기준 벡터용.
    pub fn asset_vectors(
        &self,
        conn: &Connection,
        key: &ModelKey,
        asset_id: u64,
    ) -> Result<Vec<Vec<f32>>> {
        let mut stmt = conn.prepare_cached(sql::ASSET_RAW_OFFSETS)?;
        let offsets: Vec<i64> = stmt
            .query_map(
                params![
                    asset_id as i64,
                    key.kind.as_str(),
                    key.model_id,
                    key.model_version,
                    key.dimension as i64
                ],
                |r| r.get(0),
            )?
            .collect::<std::result::Result<_, _>>()?;
        let map = self.map_raw(key)?;
        let rl = RawFile::record_len(key.dimension) as usize;
        let mut out = Vec::with_capacity(offsets.len());
        for off in offsets {
            let off = usize::try_from(off).unwrap_or(usize::MAX);
            let rec = map.as_ref().and_then(|m| m.get(off..off.checked_add(rl)?));
            let Some(rec) = rec.filter(|r| !is_hole(r)) else {
                continue;
            };
            let mut buf = Vec::with_capacity(key.dimension);
            f16_bytes_to_f32(rec, &mut buf)?;
            out.push(buf);
        }
        Ok(out)
    }
}
