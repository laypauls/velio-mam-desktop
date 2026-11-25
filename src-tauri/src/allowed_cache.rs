//! 하이브리드·유사 미디어의 "필터를 통과한 자산 집합" 짧은 캐시 (P4 후속).
//!
//! 100만 건에서 집합 구성(Tantivy 필터 질의 + id 수집)이 질의당 수백 ms 라, 같은 필터의 연속 질의(페이지 넘김·
//! 질의어 수정·스크롤)는 한 번만 구성한다. 키 = (색인 도장, 필터 전체). 도장이 바뀌면(커밋·삭제·재구축) 항목이
//! 모두 낡은 것이라 새 항목을 넣을 때 함께 버린다 — 따라서 캐시가 결과를 바꾸지 않는다. 항목 수는 `CAP` 로 제한한다
//! (집합은 자산 id 비트맵이라 100만 건에도 항목당 수백 KB).

use std::sync::{Arc, Mutex};

use mam_search::coordinator::{IndexStamp, SearchHandle};
use mam_search::{Filters, SearchError};
use mam_vector::IdSet;

/// 동시에 보관하는 필터 수 (가장 오래 안 쓴 것부터 버린다)
const CAP: usize = 4;

struct Entry {
    stamp: IndexStamp,
    filters: String,
    assets: Arc<IdSet>,
}

#[derive(Default)]
pub(crate) struct AllowedCache {
    /// 앞쪽이 가장 최근에 쓴 항목
    entries: Mutex<Vec<Entry>>,
    #[cfg(test)]
    pub(crate) builds: std::sync::atomic::AtomicUsize,
}

impl AllowedCache {
    /// `filters` 를 통과한 자산 id 집합. 캐시에 있으면 색인 질의 없이 돌려준다.
    pub(crate) fn assets(
        &self,
        handle: &SearchHandle,
        filters: &Filters,
    ) -> Result<Arc<IdSet>, SearchError> {
        let key = format!("{filters:?}");
        let stamp = handle.index_stamp()?;
        {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pos) = entries
                .iter()
                .position(|e| e.stamp == stamp && e.filters == key)
            {
                let hit = entries.remove(pos);
                let assets = Arc::clone(&hit.assets);
                entries.insert(0, hit);
                return Ok(assets);
            }
        }
        let (stamp, ids) = handle.filtered_asset_ids_stamped(filters)?;
        #[cfg(test)]
        self.builds
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let assets = Arc::new(IdSet::from_ids(&ids));
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|e| e.stamp == stamp && e.filters != key);
        entries.insert(
            0,
            Entry {
                stamp,
                filters: key,
                assets: Arc::clone(&assets),
            },
        );
        entries.truncate(CAP);
        Ok(assets)
    }
}
