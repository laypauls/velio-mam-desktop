//! 로컬 STT 전사 저장·조회·색인 텍스트 (P3-4).
//!
//! - 전사 원문은 AI 생성 설명과 **분리 저장**하고 감지 언어를 그대로 보존한다 (db.md §2).
//! - 저장은 `media_assets.revision` 증가 + `search_outbox` upsert 와 **한 트랜잭션** (db.md §3).
//! - Tantivy 에는 [`transcript_index_text`] 결과 하나를 `AssetDoc.transcript` 로 넣고, 구간 시각은 [`find_segments`] 로
//!   SQLite 에서 찾는다.

mod analysis;
mod history;
mod load;
mod save;
mod stale;

use serde::Serialize;

pub use analysis::{
    AnalysisEdit, AnalysisFields, AnalysisView, DESCRIPTION_MAX_CHARS, KEYWORD_MAX_CHARS,
    KEYWORDS_MAX, NewAnalysis, load_analysis, save_analysis, update_analysis,
};
pub use history::{
    AiUsageRow, AnalysisRun, MESSAGE_MAX_CHARS, RunRecord, USAGE_SQL, UsageEntry, list_runs,
    list_usage, record_run, record_usage,
};
pub use load::{find_segments, load_transcript, transcript_index_text};
pub use save::save_transcript;
pub use stale::{SEMANTIC_ONLY_CODE, requeue_semantic_embedding};

/// 색인 텍스트 상한 (문자 수). 긴 녹음 대비.
pub const INDEX_TEXT_MAX_CHARS: usize = 200_000;

/// AI 결과 색인은 저우선 (사용자 편집 = 0, 대량 AI = 1 - db.md §3)
pub(crate) const AI_PRIORITY: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum AiStoreError {
    /// 자산이 없거나 `active` 가 아니다 - 아무것도 바꾸지 않았다.
    #[error("asset {0} not found or not active")]
    NotFound(i64),
    /// 입력 검증 실패 (길이 초과 등) - 아무것도 바꾸지 않았다.
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, AiStoreError>;

/// 전사 구간 (UI 계약 `TranscriptSegment`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// 저장할 전사 (`mam_ai::local::stt::TranscriptResult` 에서 변환).
#[derive(Debug, Clone)]
pub struct TranscriptRecord {
    pub language: String,
    pub segments: Vec<TranscriptSegment>,
    pub engine: String,
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveOutcome {
    /// 저장으로 증가한 `media_assets.revision`
    pub revision: i64,
}

/// 조회용 전사 (UI 계약 `Transcript` + `model`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptView {
    pub language: String,
    pub text: String,
    pub segments: Vec<TranscriptSegment>,
    pub engine: String,
    pub model: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHit {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
