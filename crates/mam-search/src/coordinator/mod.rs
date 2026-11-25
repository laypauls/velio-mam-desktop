//! 검색 조정기 (P2-5) — 키워드 질의 + 사전 필터 + 정렬 + `search_after` 커서 페이지네이션.
//!
//! - 모든 필터는 Tantivy 쿼리 안에서 적용한다 (db.md §4: 후필터 금지). 필터 절은 `search` 모듈과 공유한다.
//! - 정렬은 순위 키 `[present, k1, k2, tie]` (클수록 앞) 를 직접 계산하는 `SortKeyComputer` 로 한다.
//!   `tie` 가 `asset_id` 오름차순을 보장하므로 키가 전순서이고, 커서는 페이지 경계 항목의 키다.
//!   앞 방향 다음 페이지는 키가 커서보다 **작은** 문서만, 뒤 방향(`Direction::Backward`) 이전 페이지는
//!   키가 커서보다 **큰** 문서 중 커서에 가장 가까운 `limit` 건만 후보가 된다 (offset 없음).
//! - 모든 페이지는 `start_offset`(첫 항목의 전역 0-기준 위치)과 양방향 커서를 돌려준다. 커서 페이지의 위치는
//!   커서 때문에 빠진 문서 수를 같은 수집 패스에서 세어 얻는다 (추가 패스 없음).
//! - `at` 은 전역 위치로 바로 가는 seek 이다 ([`seek`]). 커서와 함께 쓸 수 없다.
//! - 커서는 인덱스 세대와 요청 지문을 담는다. 재구축으로 세대가 바뀌거나 질의·필터·정렬이 다르면
//!   [`SearchError::StaleCursor`] — 호출자는 첫 페이지부터 다시 요청한다.
//! - 결과는 `asset_id` 목록뿐이다. 페이지 구성(하이드레이션)은 SQLite 에서 한 번에 한다 (`mam-core`).
//! - [`SearchHandle::section_index`] 는 정렬 순서대로 구간(월·이름 첫 글자)별 개수를 센다 (목차 레일용, `sections`).
//!
//! 관련도 정렬의 BM25 점수는 인덱스 통계(문서 수·빈도)에 의존하므로, 페이지 사이에 문서가 추가되면
//! 점수가 조금 달라져 중복·누락이 생길 수 있다 (필드 정렬은 영향 없음).

mod candidates;
mod collection_order;
mod handle;
mod key;
mod sections;
mod seek;

pub use candidates::IndexStamp;
pub use collection_order::CollectionOrder;
pub use handle::SearchHandle;
pub use sections::{
    Section, SectionIndex, SectionKind, local_utc_offset_secs, name_initial_bounds,
};

use std::sync::Arc;

use tantivy::Searcher;
use tantivy::collector::{Count, MultiCollector, TopDocs};
use tantivy::query::Query;

use crate::error::{Result, SearchError};
use crate::query::FilenameStrategy;
use crate::search::{Filters, SearchRequest, Sort, build_query};
use key::{
    EXCLUDED, KeyComputer, RankKey, decode_cursor, encode_cursor, fingerprint, invert_key,
    tie_to_asset,
};

/// 한 페이지 최대 건수 (UI 계약 `MAX_PAGE_LIMIT`).
pub const MAX_PAGE_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortField {
    #[default]
    Relevance,
    CapturedAt,
    /// 촬영일, 없으면 수정일 (설정 `capturedAtFallback=modified` 일 때 `CapturedAt` 대신 쓴다)
    CapturedOrModified,
    ModifiedAt,
    Name,
    /// 제목순: 제목이 있으면 제목, 없으면 파일명 기준 (설정 `titleEditEnabled` 켜짐일 때 `Name` 대신 쓴다)
    Title,
    Size,
    Duration,
    /// 별점 (`rating` fast field; 0 = 미평가도 값으로 정렬)
    Rating,
    /// 컬렉션 안 순서 (`CoordRequest::collection_order` 맵 필수)
    CollectionOrder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortSpec {
    pub field: SortField,
    pub descending: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            field: SortField::Relevance,
            descending: true,
        }
    }
}

/// 커서 기준 방향. `Backward` 는 커서 **앞**(표시 순서상 먼저) 항목을 돌려주되 결과는 표시 순서다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Forward,
    Backward,
}

#[derive(Clone, Debug)]
pub struct CoordRequest {
    /// 비어 있으면 필터·정렬만으로 탐색한다.
    pub text: String,
    pub strategy: FilenameStrategy,
    pub filters: Filters,
    pub sort: SortSpec,
    /// 1..=[`MAX_PAGE_LIMIT`]
    pub limit: usize,
    pub cursor: Option<String>,
    /// `Backward` 는 `cursor` 가 있어야 한다.
    pub direction: Direction,
    /// 전역 0-기준 위치로 seek. `cursor` 와 함께 쓸 수 없다. `at >= total` 이면 빈 페이지 (`start_offset = total`).
    pub at: Option<usize>,
    /// `SortField::CollectionOrder` 정렬에 필요한 `asset_id -> 순위` 맵. 지문(커서)에는 넣지 않는다 — 재배치 뒤에도 커서가 유효하다.
    pub collection_order: Option<Arc<CollectionOrder>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordPage {
    /// 정렬 순서대로.
    pub ids: Vec<u64>,
    /// 필터·질의에 맞는 전체 문서 수 (정확).
    pub total: u64,
    /// 이 페이지 뒤에 항목이 더 있으면 마지막 항목의 키.
    pub next_cursor: Option<String>,
    /// 이 페이지 앞에 항목이 있으면 (`start_offset > 0`) 첫 항목의 키. 빈 페이지는 항상 `None`.
    pub prev_cursor: Option<String>,
    /// `ids[0]` 의 전역 위치 (0-기준, 같은 searcher 스냅샷 기준).
    pub start_offset: u64,
    pub generation: u64,
}

/// 한 searcher 스냅샷에서 수집을 반복하기 위한 묶음.
pub(crate) struct Run<'a> {
    pub searcher: &'a Searcher,
    pub query: &'a dyn Query,
    pub sort: SortSpec,
    pub relevance: bool,
    /// `SortField::CollectionOrder` 의 순서 맵
    pub order: Option<Arc<CollectionOrder>>,
}

/// [`Run::top`] 결과. `keys` 는 수집 순서(`invert` 였다면 표시 순서의 역순)이며 원래 키로 되돌려져 있다.
pub(crate) struct Top {
    pub keys: Vec<RankKey>,
    pub total: u64,
    pub skipped: u64,
}

impl Run<'_> {
    /// 순위 상위 `k` 건 + 전체 건수 + 커서로 빠진 건수를 한 패스로 모은다.
    pub fn top(&self, k: usize, after: Option<RankKey>, invert: bool) -> Result<Top> {
        let computer =
            KeyComputer::new(self.sort, self.relevance, after, self.order.clone()).inverted(invert);
        let skipped = computer.skipped_counter();
        let mut multi = MultiCollector::new();
        let top_handle = multi.add_collector(TopDocs::with_limit(k).order_by(computer));
        let count_handle = multi.add_collector(Count);
        let mut fruits = self.searcher.search(self.query, &multi)?;
        let hits = top_handle.extract(&mut fruits);
        let total = count_handle.extract(&mut fruits) as u64;
        let keys = hits
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| *k != EXCLUDED)
            .map(|k| if invert { invert_key(&k) } else { k })
            .collect();
        Ok(Top {
            keys,
            total,
            skipped: skipped.load(std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// 전체 건수만 (정렬 키 계산 없음).
    pub fn count(&self) -> Result<u64> {
        Ok(self.searcher.search(self.query, &Count)? as u64)
    }
}

impl SearchHandle {
    /// 검색 한 페이지. 읽기 전용이며 commit 된 상태만 본다.
    pub fn search(&self, req: &CoordRequest) -> Result<CoordPage> {
        if req.limit == 0 || req.limit > MAX_PAGE_LIMIT {
            return Err(SearchError::InvalidInput(format!(
                "limit must be 1..={MAX_PAGE_LIMIT}"
            )));
        }
        if req.cursor.is_some() && req.at.is_some() {
            return Err(SearchError::InvalidInput(
                "cursor and at are mutually exclusive".into(),
            ));
        }
        if req.direction == Direction::Backward && req.cursor.is_none() {
            return Err(SearchError::InvalidInput(
                "backward paging requires a cursor".into(),
            ));
        }
        if req.sort.field == SortField::CollectionOrder && req.collection_order.is_none() {
            return Err(SearchError::InvalidInput(
                "collection order sort requires a collection order map".into(),
            ));
        }
        // 읽기 잠금을 검색이 끝날 때까지 쥔다 — 재구축의 디렉터리 교체가 진행 중인 검색을 기다린다
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        let print = fingerprint(req);
        let after = match &req.cursor {
            Some(c) => Some(decode_cursor(c, slot.generation, print)?),
            None => None,
        };

        let search_req = SearchRequest {
            text: req.text.clone(),
            strategy: req.strategy,
            filters: req.filters.clone(),
            sort: Sort::Relevance,
            limit: req.limit,
            offset: 0,
        };
        let query = build_query(&live.fields, &live.options, &search_req)?;
        let searcher = live.reader.searcher();
        let run = Run {
            searcher: &searcher,
            query: query.as_ref(),
            sort: req.sort,
            relevance: req.sort.field == SortField::Relevance && !req.text.trim().is_empty(),
            order: req.collection_order.clone(),
        };

        let limit = req.limit;
        let (keys, total, start, more) = if let Some(at) = req.at {
            seek::page(&run, at, limit)?
        } else if let (Direction::Backward, Some(after)) = (req.direction, after) {
            // 커서보다 앞(키가 큰) 문서 중 가장 가까운 limit 건 — 순서를 뒤집어 수집하고 표시 순서로 되돌린다
            let top = run.top(limit, Some(after), true)?;
            let before = top.total.saturating_sub(top.skipped);
            let mut keys = top.keys;
            keys.reverse();
            let start = before.saturating_sub(keys.len() as u64);
            let more = start + (keys.len() as u64) < top.total;
            (keys, top.total, start, more)
        } else {
            let top = run.top(limit + 1, after, false)?;
            let start = if after.is_some() { top.skipped } else { 0 };
            let mut keys = top.keys;
            let more = keys.len() > limit;
            keys.truncate(limit);
            (keys, top.total, start, more)
        };

        let generation = slot.generation;
        let next_cursor = match (more, keys.last()) {
            (true, Some(last)) => Some(encode_cursor(generation, print, last)),
            _ => None,
        };
        let prev_cursor = match (start > 0, keys.first()) {
            (true, Some(first)) => Some(encode_cursor(generation, print, first)),
            _ => None,
        };
        Ok(CoordPage {
            ids: keys.iter().map(tie_to_asset).collect(),
            total,
            next_cursor,
            prev_cursor,
            start_offset: start,
            generation,
        })
    }
}
