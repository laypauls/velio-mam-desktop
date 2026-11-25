//! 컬렉션 순서 정렬 (`SortField::CollectionOrder`) 용 `asset_id -> 순위` 맵.
//!
//! 순서는 컬렉션마다 달라 Tantivy fast field 로 둘 수 없다. 호출자(앱 계층)가 SQLite `collection_assets` 에서 읽은
//! `(asset_id, position, added_at)` 행으로 [`CollectionOrder::from_rows`] 를 만들어 요청에 실어 준다.
//! 맵은 이 요청 동안만 쓰는 읽기 전용 스냅샷이다. 색인 지연으로 맵에 없는 문서는 항상 맨 뒤다.

use std::collections::HashMap;

use super::key::ord_i64;

/// 순위 등급: 위치가 있는 행 > 위치 NULL(구버전 앱이 추가) 행 > 맵에 없는 문서(0).
pub(super) const CLASS_POSITIONED: u64 = 2;
pub(super) const CLASS_NULL: u64 = 1;

#[derive(Default)]
pub struct CollectionOrder {
    /// `asset_id -> (등급, 정렬 값)` — 값은 부호 있는 정수를 사전식 비교용으로 바꾼 것
    keys: HashMap<u64, (u64, u64)>,
}

impl std::fmt::Debug for CollectionOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CollectionOrder({} rows)", self.keys.len())
    }
}

impl CollectionOrder {
    /// `(asset_id, position, added_at)` 행에서 만든다. `position` 이 NULL 이면 `added_at` 순으로 위치 있는 행 뒤에 둔다.
    pub fn from_rows(rows: impl IntoIterator<Item = (i64, Option<i64>, i64)>) -> Self {
        let keys = rows
            .into_iter()
            .filter(|(asset_id, ..)| *asset_id > 0)
            .map(|(asset_id, position, added_at)| {
                let key = match position {
                    Some(p) => (CLASS_POSITIONED, ord_i64(p)),
                    None => (CLASS_NULL, ord_i64(added_at)),
                };
                (asset_id as u64, key)
            })
            .collect();
        Self { keys }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub(super) fn get(&self, asset_id: u64) -> Option<(u64, u64)> {
        self.keys.get(&asset_id).copied()
    }
}
