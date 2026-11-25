//! 검색 요청·실행. 결과는 `asset_id`(+점수)만 돌려준다 — 상세는 최종 후보에 대해서만 SQLite 에서 조회한다.

use std::collections::HashMap;
use std::ops::Bound;

use tantivy::collector::TopDocs;
use tantivy::query::{
    AllQuery, BooleanQuery, ConstScoreQuery, EmptyQuery, Occur, Query, RangeQuery, TermQuery,
    TermSetQuery,
};
use tantivy::schema::IndexRecordOption;
use tantivy::{DocAddress, Order, Term};

use crate::doc::{MediaType, Presence, dir_token};
use crate::error::{Result, SearchError};
use crate::index::{IndexOptions, SearchIndex};
use crate::query::{FilenameStrategy, word_clause};
use crate::schema::Fields;
use crate::userfield_filter::UserFieldCondition;

/// 한 번에 가져올 수 있는 최대 건수.
pub const MAX_LIMIT: usize = 1000;
/// `offset + limit` 의 상한. Tantivy 는 `offset + limit` 만큼 힙을 유지하므로 깊은 페이징은
/// 비용이 선형으로 늘어난다 — 더 깊은 탐색은 search_after 방식(P2)이 필요하다.
pub const MAX_WINDOW: usize = 10_000;

/// 닫힌 구간 `[min, max]`. 한쪽이 `None` 이면 그쪽은 열려 있다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range<T> {
    pub min: Option<T>,
    pub max: Option<T>,
}

#[derive(Clone, Debug, Default)]
pub struct Filters {
    /// 비어 있으면 필터 없음.
    pub media_types: Vec<MediaType>,
    /// `None` = 볼륨 필터 없음, `Some(빈 집합)` = **결과 없음**. 온라인 볼륨 id 집합을
    /// SQLite 에서 읽어 넘긴다 (결정 D2).
    pub online_volume_ids: Option<Vec<u64>>,
    pub presence: Option<Presence>,
    pub captured_at: Range<i64>,
    pub file_size: Range<u64>,
    pub duration_ms: Range<u64>,
    /// 폴더 범위 (P2-5). 색인 토큰으로 맞춘다.
    pub folder: Option<FolderFilter>,
    /// 즐겨찾기만 (P2-5)
    pub favorite_only: bool,
    /// 별점 범위 (포함, 0 = 미평가) — 검색 단계의 RangeQuery
    pub rating: Range<u64>,
    /// 이 컬렉션에 속한 자산만 (P2-5)
    pub collection_id: Option<u64>,
    /// 모든 태그를 가진 자산 (AND) (P2-5)
    pub tag_ids: Vec<u64>,
    /// 사용자 제목(`title`)도 키워드 검색 대상에 넣는다. 설정 `titleEditEnabled` 와 같은 값 — 꺼져 있으면(기본)
    /// 저장된 제목은 색인에 있어도 질의에 전혀 쓰이지 않는다 (재색인 없이 토글).
    pub search_title: bool,
    /// 촬영일 범위 필터(와 `Sort::CapturedAt*`)가 `captured_or_modified`(촬영일 없으면 수정일)를 쓴다.
    /// 설정 `capturedAtFallback=modified` 와 같은 값 — 꺼져 있으면 원래 `captured_at` (재색인 없이 토글).
    /// 필터 지문에 포함되므로 값이 바뀌면 기존 커서는 stale 이 된다.
    pub captured_fallback: bool,
    /// 검색에서 제외할 관리 폴더 id (숨김 폴더). 비어 있으면 제외 없음. 색인 문서의 `dir_anc` 루트 토큰에 대한
    /// `MustNot` 으로 질의 시점에 걸리므로 재색인이 필요 없다. 필터 지문에 포함되어 값이 바뀌면 기존 커서는 stale 이 된다.
    pub hidden_folder_ids: Vec<u64>,
    /// 사용자 정의 메타데이터 필드 조건 (서로 AND). 검색 단계의 사전 필터로 걸린다 — 형식은 `UserFieldCondition` 참고.
    /// 필드 정의와의 대조(삭제된 필드·선택지, 타입 불일치)는 서비스 계층 몫이다. 필터 지문(Debug)에 포함되어 바뀌면 기존 커서는 stale 이 된다.
    pub user_fields: Vec<UserFieldCondition>,
}

impl Filters {
    /// 촬영일 정렬·필터가 읽을 fast field 이름.
    pub(crate) fn captured_field_name(&self) -> &'static str {
        if self.captured_fallback {
            "captured_or_modified"
        } else {
            "captured_at"
        }
    }
}

/// 관리 폴더 안의 범위. `dir_key` 는 폴더 루트 기준 `path_key` 형식(소문자, `/` 구분, 루트 = `""`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderFilter {
    pub folder_id: u64,
    pub dir_key: String,
    pub recursive: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// BM25 (필드 가중치 포함) 내림차순.
    #[default]
    Relevance,
    /// fast field 정렬. 값이 없는 문서는 오름·내림 모두 뒤로 간다.
    CapturedAtDesc,
    CapturedAtAsc,
    DurationMsDesc,
}

#[derive(Clone, Debug)]
pub struct SearchRequest {
    /// 공백으로 나눈 단어 모두 일치(AND). 비어 있으면 필터만으로 조회한다.
    pub text: String,
    pub strategy: FilenameStrategy,
    pub filters: Filters,
    pub sort: Sort,
    pub limit: usize,
    pub offset: usize,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            text: String::new(),
            strategy: FilenameStrategy::A,
            filters: Filters::default(),
            sort: Sort::Relevance,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub asset_id: u64,
    /// 관련도 정렬일 때 BM25 합, 그 외 0.
    pub score: f32,
}

fn bounds<T: Copy>(r: &Range<T>, mk: impl Fn(T) -> Term) -> (Bound<Term>, Bound<Term>) {
    (
        r.min.map_or(Bound::Unbounded, |v| Bound::Included(mk(v))),
        r.max.map_or(Bound::Unbounded, |v| Bound::Included(mk(v))),
    )
}

fn term_u64(f: tantivy::schema::Field, v: u64) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_u64(f, v),
        IndexRecordOption::Basic,
    ))
}

/// 필터 절들. `None` = 결과 없음이 확정(온라인 볼륨 없음).
pub(crate) fn filter_clauses(f: &Fields, flt: &Filters) -> Option<Vec<Box<dyn Query>>> {
    {
        let mut out: Vec<Box<dyn Query>> = Vec::new();
        if !flt.media_types.is_empty() {
            let terms = flt
                .media_types
                .iter()
                .map(|m| Term::from_field_text(f.media_type, m.as_str()));
            out.push(Box::new(TermSetQuery::new(terms)));
        }
        if let Some(ids) = &flt.online_volume_ids {
            if ids.is_empty() {
                return None; // 온라인 볼륨 없음 → 결과 없음
            }
            out.push(Box::new(TermSetQuery::new(
                ids.iter().map(|v| Term::from_field_u64(f.volume_id, *v)),
            )));
        }
        if let Some(p) = flt.presence {
            out.push(Box::new(TermQuery::new(
                Term::from_field_text(f.presence, p.as_str()),
                IndexRecordOption::Basic,
            )));
        }
        let captured_field = if flt.captured_fallback {
            f.captured_or_modified
        } else {
            f.captured_at
        };
        let ranges = [
            (flt.captured_at != Range::default()).then(|| {
                bounds(&flt.captured_at, |v| {
                    Term::from_field_i64(captured_field, v)
                })
            }),
            (flt.file_size != Range::default())
                .then(|| bounds(&flt.file_size, |v| Term::from_field_u64(f.file_size, v))),
            (flt.duration_ms != Range::default())
                .then(|| bounds(&flt.duration_ms, |v| Term::from_field_u64(f.duration_ms, v))),
            (flt.rating != Range::default())
                .then(|| bounds(&flt.rating, |v| Term::from_field_u64(f.rating, v))),
        ];
        for (lo, hi) in ranges.into_iter().flatten() {
            out.push(Box::new(RangeQuery::new(lo, hi)));
        }
        if let Some(fl) = &flt.folder {
            let field = if fl.recursive { f.dir_anc } else { f.dir_exact };
            out.push(Box::new(TermQuery::new(
                Term::from_field_text(field, &dir_token(fl.folder_id, &fl.dir_key)),
                IndexRecordOption::Basic,
            )));
        }
        if flt.favorite_only {
            out.push(term_u64(f.favorite, 1));
        }
        if let Some(c) = flt.collection_id {
            out.push(term_u64(f.collection_id, c));
        }
        for t in &flt.tag_ids {
            out.push(term_u64(f.tag_id, *t));
        }
        for c in &flt.user_fields {
            out.push(c.to_query(f));
        }
        Some(out)
    }
}

/// 질의 + 필터 -> Tantivy 쿼리.
pub(crate) fn build_query(
    fields: &Fields,
    options: &IndexOptions,
    req: &SearchRequest,
) -> Result<Box<dyn Query>> {
    {
        use FilenameStrategy as S;
        match req.strategy {
            S::B if !options.build_ngram => {
                return Err(SearchError::InvalidInput(
                    "strategy B needs an index built with build_ngram".into(),
                ));
            }
            S::C if !options.build_raw => {
                return Err(SearchError::InvalidInput(
                    "strategy C needs an index built with build_raw".into(),
                ));
            }
            _ => {}
        }
        for c in &req.filters.user_fields {
            c.validate()?;
        }
        let Some(filters) = filter_clauses(fields, &req.filters) else {
            return Ok(Box::new(EmptyQuery));
        };
        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        if req.text.trim().is_empty() {
            clauses.push((Occur::Must, Box::new(AllQuery)));
        } else {
            let words: Vec<&str> = req.text.split_whitespace().collect();
            let mut any = false;
            for w in words {
                // 분석 후 토큰이 없는 단어(기호만 등)는 무시한다
                if let Some(q) = word_clause(fields, w, req.strategy, req.filters.search_title)? {
                    clauses.push((Occur::Must, q));
                    any = true;
                }
            }
            if !any {
                return Ok(Box::new(EmptyQuery)); // 쓸 수 있는 단어가 하나도 없으면 결과 없음
            }
        }
        for q in filters {
            clauses.push((Occur::Must, Box::new(ConstScoreQuery::new(q, 0.0))));
        }
        for id in &req.filters.hidden_folder_ids {
            clauses.push((
                Occur::MustNot,
                Box::new(TermQuery::new(
                    Term::from_field_text(fields.dir_anc, &dir_token(*id, "")),
                    IndexRecordOption::Basic,
                )),
            ));
        }
        Ok(Box::new(BooleanQuery::new(clauses)))
    }
}

impl SearchIndex {
    /// 검색. 읽기 전용이며 `commit` 된 상태만 본다.
    pub fn search(&self, req: &SearchRequest) -> Result<Vec<Hit>> {
        if req.limit == 0 {
            return Ok(Vec::new());
        }
        if req.limit > MAX_LIMIT {
            return Err(SearchError::InvalidInput(format!(
                "limit {} exceeds MAX_LIMIT {MAX_LIMIT}",
                req.limit
            )));
        }
        match req.offset.checked_add(req.limit) {
            Some(w) if w <= MAX_WINDOW => {}
            _ => {
                return Err(SearchError::InvalidInput(format!(
                    "offset + limit must be <= {MAX_WINDOW} (deep paging needs search_after)"
                )));
            }
        }
        let query = build_query(&self.fields, &self.options, req)?;
        let searcher = self.reader.searcher();
        let top = TopDocs::with_limit(req.limit).and_offset(req.offset);
        let addrs: Vec<(f32, DocAddress)> = match req.sort {
            Sort::Relevance => searcher.search(&query, &top.order_by_score())?,
            Sort::CapturedAtDesc => with_zero(searcher.search(
                &query,
                &top.order_by_fast_field::<i64>(req.filters.captured_field_name(), Order::Desc),
            )?),
            Sort::CapturedAtAsc => with_zero(searcher.search(
                &query,
                &top.order_by_fast_field::<i64>(req.filters.captured_field_name(), Order::Asc),
            )?),
            Sort::DurationMsDesc => with_zero(searcher.search(
                &query,
                &top.order_by_fast_field::<u64>("duration_ms", Order::Desc),
            )?),
        };
        let mut columns = HashMap::new();
        let mut hits = Vec::with_capacity(addrs.len());
        for (score, addr) in addrs {
            let col = match columns.entry(addr.segment_ord) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(
                    searcher
                        .segment_reader(addr.segment_ord)
                        .fast_fields()
                        .u64("asset_id")?,
                ),
            };
            if let Some(asset_id) = col.first(addr.doc_id) {
                hits.push(Hit { asset_id, score });
            }
        }
        Ok(hits)
    }
}

fn with_zero<K>(v: Vec<(K, DocAddress)>) -> Vec<(f32, DocAddress)> {
    v.into_iter().map(|(_, a)| (0.0, a)).collect()
}
