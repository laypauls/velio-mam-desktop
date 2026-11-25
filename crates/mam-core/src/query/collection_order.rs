//! 컬렉션 순서 정렬용 순서 맵 원천 — 조정기(`CollectionOrder`)가 `asset_id -> 순위` 로 바꾼다.

use super::QueryService;
use crate::scan::IngestError;

impl QueryService {
    /// 컬렉션의 `(asset_id, position, added_at)` 전 행. 없는 컬렉션은 빈 목록 (호출자가 정렬 결과를 비운다).
    /// `position` NULL = 이전 앱이 추가한 행 (위치 있는 행 뒤, `added_at` 순).
    pub fn collection_order_rows(
        &self,
        collection_id: i64,
    ) -> Result<Vec<(i64, Option<i64>, i64)>, IngestError> {
        self.read(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT asset_id, position, added_at FROM collection_assets WHERE collection_id = ?1",
            )?;
            let rows = stmt.query_map([collection_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }
}
