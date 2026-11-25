//! 자산 임베딩 작업 함수 (P3-5) — 시각(CLIP 이미지/프레임)·의미(e5 텍스트) 벡터를 만들어 보관한다.
//!
//! - 인코더는 trait ([`VisualEncoder`], [`TextEncoder`]) 로 주입한다 — `mam-core` 는 ONNX(`mam-ai`)에 의존하지 않는다.
//!   실제 어댑터는 `mam-ai` 임베더를 감싸 수 줄로 구현한다.
//! - 저장은 `mam_vector::EmbedStore` (f16 raw 파일 + SQLite `embeddings`). 같은 (자산, 종류, 모델)의 기존 벡터는
//!   한 트랜잭션에서 폐기하고 새로 저장한다.
//! - 이미지 입력은 MAMData 안의 썸네일·프레임 파일만 읽는다 — 원본 미디어 경로는 거부한다 (원본 보호).
//! - 작업 대기열 등록·Tauri 연결은 이 모듈 밖(백엔드)에서 한다. 시계는 `now` 로 주입한다.

mod encoder;
mod job;

pub use encoder::{ModelInfo, TextEncoder, VisualEncoder};
pub use job::{
    ImageSource, PASSAGE_PREFIX, QUERY_PREFIX, SEMANTIC_TEXT_MAX_CHARS, embed_query,
    embed_semantic, embed_visual,
};

use std::path::PathBuf;

/// 임베딩 작업 오류. 인코더 실패는 문자열로 보존한다 (재시도 정책은 호출자).
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// 이미지 경로가 MAMData 밖이거나 `..` 를 포함한다 — 원본 미디어는 읽지 않는다.
    #[error("image path is not inside the MAM data directory: {}", .0.display())]
    OutsideDataDir(PathBuf),
    #[error("encoder error: {0}")]
    Encoder(String),
    #[error("encoder returned {found} vectors for {expected} inputs")]
    CountMismatch { expected: usize, found: usize },
    #[error("encoder vector has dimension {found}, model declares {expected}")]
    DimensionMismatch { expected: usize, found: usize },
    #[error("vector store error: {0}")]
    Vector(#[from] mam_vector::VectorError),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}
