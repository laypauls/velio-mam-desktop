//! 하이브리드 검색용 후보 질의 (P3) — 키워드 상위 N 과 필터를 통과한 자산 집합.
//!
//! 벡터 쪽은 "필터를 먼저 적용한 허용 집합 안에서" 후보를 찾아야 하므로(db.md §4) 질의 텍스트와 무관하게
//! 필터만으로 자산 id 를 모으는 [`SearchHandle::filtered_asset_ids`] 가 필요하다.

use std::hash::{Hash, Hasher};

use tantivy::collector::{Collector, SegmentCollector, TopDocs};
use tantivy::{DocId, Score, SegmentReader};

use super::key::{EXCLUDED, KeyComputer, tie_to_asset};
use super::{SearchHandle, SortSpec};
use crate::error::{Result, SearchError};
use crate::query::FilenameStrategy;
use crate::search::{Filters, MAX_WINDOW, SearchRequest, Sort, build_query};

impl SearchHandle {
    /// 질의에 맞는 상위 `n`(<= [`MAX_WINDOW`]) 개의 자산 id (BM25 내림차순, 동점은 asset_id 오름차순). 필터는 질의 안에서 적용한다.
    pub fn keyword_candidates(
        &self,
        text: &str,
        strategy: FilenameStrategy,
        filters: &Filters,
        n: usize,
    ) -> Result<Vec<u64>> {
        if n > MAX_WINDOW {
            return Err(SearchError::InvalidInput(format!(
                "candidate count must be <= {MAX_WINDOW}"
            )));
        }
        if n == 0 || text.trim().is_empty() {
            return Ok(Vec::new());
        }
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        let req = SearchRequest {
            text: text.to_string(),
            strategy,
            filters: filters.clone(),
            sort: Sort::Relevance,
            limit: n,
            offset: 0,
        };
        let query = build_query(&live.fields, &live.options, &req)?;
        let searcher = live.reader.searcher();
        // 정렬은 조정기와 같은 순위 키(관련도 -> asset_id 오름차순)다 - 동점은 asset_id 로 정해져 세그먼트 배치와 무관하다
        let computer = KeyComputer::new(SortSpec::default(), true, None, None);
        let top = searcher.search(&query, &TopDocs::with_limit(n).order_by(computer))?;
        Ok(top
            .into_iter()
            .map(|(k, _)| k)
            .filter(|k| *k != EXCLUDED)
            .map(|k| tie_to_asset(&k))
            .collect())
    }

    /// 필터를 통과한 모든 자산 id (순서 없음, 질의 텍스트 없음).
    pub fn filtered_asset_ids(&self, filters: &Filters) -> Result<Vec<u64>> {
        Ok(self.filtered_asset_ids_stamped(filters)?.1)
    }

    /// [`Self::filtered_asset_ids`] + 그 결과를 만든 색인 상태의 도장. 같은 필터로 도장이 같으면 결과가 같으므로
    /// 호출자가 (도장, 필터)로 결과를 잠깐 재사용할 수 있다 (도장은 같은 searcher 에서 읽는다).
    pub fn filtered_asset_ids_stamped(&self, filters: &Filters) -> Result<(IndexStamp, Vec<u64>)> {
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        let req = SearchRequest {
            filters: filters.clone(),
            limit: 1,
            ..SearchRequest::default()
        };
        let query = build_query(&live.fields, &live.options, &req)?;
        let searcher = live.reader.searcher();
        let stamp = stamp_of(slot.generation, &searcher);
        let ids = searcher.search(&query, &AssetIdCollector)?;
        Ok((stamp, ids))
    }

    /// [`Self::filtered_asset_ids`] 의 기준 구현 (문서 주소 집합 `DocSetCollector` 로 모은 뒤 id 를 읽는다) - 새 수집기와의
    /// 동치 시험과 측정의 "이전 방식" 용이다. 제품 경로는 쓰지 않는다.
    #[doc(hidden)]
    pub fn filtered_asset_ids_docset(&self, filters: &Filters) -> Result<Vec<u64>> {
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        let req = SearchRequest {
            filters: filters.clone(),
            limit: 1,
            ..SearchRequest::default()
        };
        let query = build_query(&live.fields, &live.options, &req)?;
        let searcher = live.reader.searcher();
        let docs = searcher.search(&query, &tantivy::collector::DocSetCollector)?;
        // 세그먼트별 열을 한 번만 연다 (이전 제품 구현과 같다)
        let mut columns: Vec<Option<tantivy::columnar::Column<u64>>> = Vec::new();
        let mut ids = Vec::with_capacity(docs.len());
        for addr in docs {
            let ord = addr.segment_ord as usize;
            if columns.len() <= ord {
                columns.resize_with(ord + 1, || None);
            }
            if columns[ord].is_none() {
                columns[ord] = Some(
                    searcher
                        .segment_reader(addr.segment_ord)
                        .fast_fields()
                        .u64("asset_id")?,
                );
            }
            if let Some(id) = columns[ord].as_ref().and_then(|c| c.first(addr.doc_id)) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// 현재 색인 상태의 도장 (재구축·커밋·삭제 반영 때마다 바뀐다). 색인이 없으면 `NotReady`.
    pub fn index_stamp(&self) -> Result<IndexStamp> {
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        Ok(stamp_of(slot.generation, &live.reader.searcher()))
    }
}

/// 색인 상태 식별자 = 세대 + 세그먼트 목록(id·문서 수·삭제 수). 문서가 바뀌는 커밋은 세그먼트 구성을 바꾼다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IndexStamp(u64);

fn stamp_of(generation: u64, searcher: &tantivy::Searcher) -> IndexStamp {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    generation.hash(&mut h);
    for s in searcher.segment_readers() {
        s.segment_id().hash(&mut h);
        s.max_doc().hash(&mut h);
        s.num_deleted_docs().hash(&mut h);
    }
    IndexStamp(h.finish())
}

/// 문서의 `asset_id` fast field 값을 점수 계산 없이 모은다 (`DocSetCollector` 의 문서 주소 `HashSet` 100만 개를 피한다).
struct AssetIdCollector;

struct AssetIdSegment {
    column: tantivy::columnar::Column<u64>,
    ids: Vec<u64>,
}

impl Collector for AssetIdCollector {
    type Fruit = Vec<u64>;
    type Child = AssetIdSegment;

    fn for_segment(
        &self,
        _segment_ord: u32,
        segment: &SegmentReader,
    ) -> tantivy::Result<AssetIdSegment> {
        Ok(AssetIdSegment {
            column: segment.fast_fields().u64("asset_id")?,
            ids: Vec::new(),
        })
    }

    fn requires_scoring(&self) -> bool {
        false
    }

    fn merge_fruits(&self, fruits: Vec<Vec<u64>>) -> tantivy::Result<Vec<u64>> {
        let mut all = Vec::with_capacity(fruits.iter().map(Vec::len).sum());
        fruits.into_iter().for_each(|f| all.extend(f));
        Ok(all)
    }
}

impl SegmentCollector for AssetIdSegment {
    type Fruit = Vec<u64>;

    fn collect(&mut self, doc: DocId, _score: Score) {
        if let Some(id) = self.column.first(doc) {
            self.ids.push(id);
        }
    }

    fn harvest(self) -> Vec<u64> {
        self.ids
    }
}
