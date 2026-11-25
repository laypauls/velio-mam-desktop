//! 활성 맵 캐시 — 질의마다 `embeddings` 활성 행 전체(220만 행 1.3~1.7초)를 다시 읽지 않는다.
//!
//! 키: (종류, 모델 ID, 버전, 차원) + 저장소 루트, 항목에 활성 세대 이름을 기억한다. 질의는 `Arc<ActiveMap>` 스냅샷만 읽는다.
//!
//! **무효화 — [`ActiveMapCache::get_or_load`] 가 스스로 판정하는 경우 (명시 호출 불필요)**
//! 1. 활성 세대 이름이 바뀌었다 (재구축·모델 교체) -> 전체 재로드. 질의당 `ACTIVE` 포인터 파일 읽기 1회.
//! 2. `EmbedStore::put` 이 커밋마다 (키별) 변경 로그에 순번과 자산 id 를 남긴다 ([`crate::ChangeEpoch`]).
//!    순번이 그대로면 DB 를 보지 않는다 — 질의당 비용 = 포인터 읽기 + 뮤텍스 몇 개 + 로그 조회 (수십~수백 µs).
//! 3. 순번이 달라졌으면 **바뀐 자산만** DB 에서 다시 읽어(`idx_embeddings_asset`, 자산당 1회) 맵에서 그 자산의 행을
//!    모두 교체한다 — 새 행 추가·재임베딩으로 인한 폐기·폐기만 하는 `put`(빈 벡터)을 모두 같은 경로로 처리한다.
//!    시각(`superseded_at`)이나 행 수 비교에 의존하지 않아 시계가 거꾸로 가도 어긋나지 않는다.
//!    비용은 바뀐 자산 수 + 맵 한 번 병합(220만 행 수십 ms 이하)이다. 바뀐 자산이 너무 많거나 로그가 잘렸으면 전체 재로드.
//!
//! **명시 호출이 필요한 경우** — `put` 을 거치지 않는 변경: 자산 영구 삭제(FK CASCADE)·데이터 초기화·직접 SQL.
//! `EmbedStore::notify_changed` 또는 [`ActiveMapCache::invalidate`] 를 부른다.
//! 다른 프로세스가 쓰는 경우는 지원하지 않는다 (변경 로그는 프로세스 안에서만 공유된다).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use crate::active_map::ActiveMap;
use crate::embed_state::{self, ChangeEpoch};
use crate::embed_store::{EmbedStore, ModelKey};
use crate::error::Result;
use crate::generation::Generations;
use crate::idset::IdSet;
use crate::sql;

/// 이보다 많은 자산이 바뀌었으면 증분(자산당 질의) 대신 전체 재로드가 싸다.
const MAX_INCREMENTAL_ASSETS: usize = 50_000;

struct Entry {
    generation: Option<String>,
    epoch: ChangeEpoch,
    map: Arc<ActiveMap>,
}

type Slot = Arc<Mutex<Option<Entry>>>;

#[derive(Default)]
pub struct ActiveMapCache {
    slots: Mutex<HashMap<String, Slot>>,
}

fn slot_name(store: &EmbedStore, key: &ModelKey) -> String {
    format!("{}|{}", store.key_dir(key).display(), key.dimension)
}

impl ActiveMapCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn slot(&self, store: &EmbedStore, key: &ModelKey) -> Slot {
        let mut g = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        g.entry(slot_name(store, key)).or_default().clone()
    }

    /// 모든 항목을 버린다 (다음 질의가 전체 재로드).
    pub fn invalidate_all(&self) {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 한 키의 항목을 버린다.
    pub fn invalidate(&self, store: &EmbedStore, key: &ModelKey) {
        let mut g = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        g.remove(&slot_name(store, key));
    }

    /// 활성 맵 스냅샷. 첫 호출·세대 교체·로그 유실 때만 전체 로드(O(활성 행))다 — 앱 시작 후 백그라운드에서
    /// 한 번 불러 데워 두면 첫 질의가 느리지 않다. 한 키가 로드 중이면 같은 키의 호출만 기다린다.
    pub fn get_or_load(
        &self,
        conn: &Connection,
        store: &EmbedStore,
        key: &ModelKey,
    ) -> Result<Arc<ActiveMap>> {
        let generation = Generations::new(store, key)?.active_name()?;
        let epoch = store.change_epoch(key); // 쿼리보다 먼저 읽는다 — 그 사이 커밋은 다음 호출이 다시 본다
        let slot = self.slot(store, key);
        let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = g.as_mut()
            && e.generation == generation
        {
            if e.epoch == epoch {
                return Ok(e.map.clone());
            }
            if refresh(conn, store, key, e, epoch)? {
                e.epoch = epoch;
                return Ok(e.map.clone());
            }
        }
        let map = Arc::new(load_full(conn, key)?);
        *g = Some(Entry {
            generation,
            epoch,
            map: map.clone(),
        });
        Ok(map)
    }
}

fn load_full(conn: &Connection, key: &ModelKey) -> Result<ActiveMap> {
    // 활성 행을 임베딩 id 순으로 (V7 커버링 인덱스 — 테이블 조회·정렬 없음)
    let mut stmt = conn.prepare_cached(sql::ACTIVE_MAP_LOAD)?;
    let rows = stmt.query_map(
        params![
            key.kind.as_str(),
            key.model_id,
            key.model_version,
            key.dimension as i64
        ],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
    )?;
    let (ids, assets): (Vec<u64>, Vec<u64>) = rows.collect::<std::result::Result<_, _>>()?;
    Ok(ActiveMap::from_sorted(ids, assets))
}

/// 바뀐 자산만 반영한다. 증분이 불가능하면(리셋 통지·로그 유실·너무 많은 자산) `false` — 호출자가 전체 재로드.
fn refresh(
    conn: &Connection,
    store: &EmbedStore,
    key: &ModelKey,
    e: &mut Entry,
    now: ChangeEpoch,
) -> Result<bool> {
    if e.epoch.resets != now.resets {
        return Ok(false);
    }
    let dir = store.key_dir(key);
    let Some(mut touched) = embed_state::touched_between(&dir, e.epoch.seq, now.seq) else {
        return Ok(false);
    };
    touched.sort_unstable();
    touched.dedup();
    if touched.len() > MAX_INCREMENTAL_ASSETS {
        return Ok(false);
    }
    let mut rows = Vec::new();
    {
        let tx = conn.unchecked_transaction()?; // 모든 자산을 같은 스냅샷에서 읽는다
        let mut stmt = tx.prepare_cached(sql::ASSET_ACTIVE_IDS)?;
        for &asset in &touched {
            let ids = stmt.query_map(
                params![
                    asset as i64,
                    key.kind.as_str(),
                    key.model_id,
                    key.model_version,
                    key.dimension as i64
                ],
                |r| r.get::<_, i64>(0).map(|v| v as u64),
            )?;
            for id in ids {
                rows.push((id?, asset));
            }
        }
    }
    rows.sort_unstable();
    Arc::make_mut(&mut e.map).replace_assets(&IdSet::from_ids(&touched), rows); // 스냅샷을 쥔 질의가 있으면 복사된다
    Ok(true)
}
