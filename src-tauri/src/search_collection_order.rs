//! `collectionOrder` 정렬 — 컬렉션 안 순서 맵을 SQLite 에서 읽어 조정기 요청에 싣는다.
//!
//! 순서는 Outbox/색인 대상이 아니다 (재배치는 revision 을 올리지 않는다). 그래서 요청마다 `collection_assets` 에서
//! 읽는다 — 재배치 직후 첫 조회부터 새 순서가 보인다. 읽기 연결이라 쓰기 큐를 막지 않는다.

use std::sync::Arc;

use mam_core::scan::IngestError;
use mam_search::coordinator::{CollectionOrder, CoordRequest, SortField};

use crate::search_service::{SearchFailure, SearchService};

impl SearchService {
    /// `coord.sort` 가 `CollectionOrder` 이면 `collection_id` 컬렉션의 순서 맵을 `coord` 에 싣는다.
    /// 다른 정렬이면 아무것도 하지 않는다.
    pub(crate) fn attach_collection_order(
        &self,
        coord: &mut CoordRequest,
        collection_id: Option<i64>,
    ) -> Result<(), SearchFailure> {
        if coord.sort.field != SortField::CollectionOrder {
            return Ok(());
        }
        // coord_request 가 이미 검증하지만, 맵 없이 정렬이 실행되지 않도록 한 번 더 막는다
        let cid = collection_id.ok_or(IngestError::InvalidInput("sort"))?;
        let rows = self.query.collection_order_rows(cid)?;
        coord.collection_order = Some(Arc::new(CollectionOrder::from_rows(rows)));
        Ok(())
    }
}
