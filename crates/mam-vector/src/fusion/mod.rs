//! 하이브리드 결합 코어 (P3-6) — RRF 순위 결합, 후보 수 계획·필터 전략, 벡터 후보 수집, 유사 미디어 질의.
//!
//! 순수 계산이다: SQLite·Tantivy·Tauri 를 모른다. 호출자(검색 조정기)가 키워드 후보(자산 id, 순위순)와
//! 필터 허용 집합을 넘긴다. BM25 와 코사인 점수는 서로 더하지 않고 **순위만** 쓴다 (`.claude/rules/db.md` §4).

mod candidates;
mod plan;
mod rrf;
mod similar;

pub use candidates::vector_candidates;
pub use candidates::vector_candidates_with_set;
pub use plan::{
    CandidatePlan, FilterStrategy, MAX_PER_SOURCE, MIN_PER_SOURCE, SUBSET_SELECTIVITY,
    choose_filter_strategy, needs_more, plan_candidates,
};
pub use rrf::{FusedHit, HybridOutcome, RrfParams, fuse_hybrid, fuse_rrf};
pub use similar::{MAX_SIMILAR_LIMIT, SimilarCursor, SimilarPage, SimilarQuery, find_similar};
