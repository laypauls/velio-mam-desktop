//! MAM Desktop 벡터 인덱스 PoC (P1-3) — 후보 엔진 비교, 원시 임베딩 저장, 측정 도구용 라이브러리.
//!
//! - `flat` : 전수 비교 (유일한 엔진 — P1-3 결론), f32/f16/i8, 메모리 매핑
//! - `fusion` : 하이브리드 결합 코어 (P3-6) — RRF, 후보 계획, 필터 우선 벡터 후보, 유사 미디어 질의
//!
//! 거리 함수는 코사인 하나다 (입력을 정규화). 인덱스는 파생 데이터이므로 항상 [`raw`] 의 원시 임베딩으로
//! 재구축할 수 있어야 한다.

mod active_cache;
mod active_map;
mod embed_state;
pub mod embed_store;
mod embed_types;
pub mod error;
pub mod flat;
mod flat_file;
mod flat_multi;
pub mod fusion;
pub mod generation;
pub mod group;
mod idset;
pub mod kernel;
mod meta;
pub mod raw;
mod scan;
mod sql;
pub mod store;
mod store_version;
pub mod swap;
mod topk;
pub mod types;

pub use active_cache::ActiveMapCache;
pub use active_map::ActiveMap;
pub use embed_state::ChangeEpoch;
pub use embed_store::{EmbedStore, ModelKey, NewVector, ReadReport, VectorKind, VectorSource};
pub use error::{Result, VectorError};
pub use flat::FlatIndex;
pub use fusion::{
    CandidatePlan, FilterStrategy, FusedHit, HybridOutcome, RrfParams, SimilarCursor, SimilarPage,
    SimilarQuery, choose_filter_strategy, find_similar, fuse_hybrid, fuse_rrf, needs_more,
    plan_candidates, vector_candidates, vector_candidates_with_set,
};
pub use generation::{
    Generations, RebuildReport, active_candidates, active_candidates_cached, index_spec,
    sync_incremental,
};
pub use group::{
    AssetHit, AssetSearch, PostFiltered, group_by_asset, search_assets, search_post_filter,
};
pub use idset::IdSet;
pub use meta::FORMAT_VERSION;
pub use scan::exact_topk_f32;
pub use store::{Engine, IndexSpec, create_index, open_index, save_index};
pub use store_version::{data_file_versions, save_index_version};
pub use swap::IndexRoot;
pub use types::{
    AnnIndex, EmbeddingId, Filter, Hit, MAX_K, OpenMode, Quantization, SearchParams, add_parallel,
    normalized,
};
