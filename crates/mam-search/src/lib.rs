//! MAM 풀텍스트 검색 — Tantivy 스키마·분석기(Lindera ko-dic, 파일명 전처리)·색인·검색.
//!
//! Tantivy 인덱스는 SQLite 로 재구축 가능한 **파생 데이터**다. 인덱스 계층(`index`·`search`)은
//! SQLite·Tauri 에 의존하지 않고 `AssetDoc` 만 받는다. [`sync`] 모듈이 SQLite Outbox 를 읽어
//! `AssetDoc` 을 만들고 인덱스에 반영하는 색인 워커다.

pub mod analyzer;
pub mod coordinator;
pub mod doc;
pub mod error;
pub mod index;
pub mod korean;
pub mod meta;
pub mod preprocess;
pub mod query;
pub mod retry_dir;
pub mod schema;
pub mod search;
pub mod sync;
pub mod userfield;
pub mod userfield_filter;

pub use doc::{AssetDoc, MediaType, Presence};
pub use error::{Result, SearchError};
pub use index::{IndexOptions, SearchIndex, UpsertOutcome};
pub use query::FilenameStrategy;
pub use retry_dir::{RetryDirectory, RetryPolicy};
pub use userfield::{UserFieldValue, UserValue, keyword_key};
pub use userfield_filter::UserFieldCondition;
pub use search::{Filters, Hit, MAX_LIMIT, MAX_WINDOW, Range, SearchRequest, Sort};
