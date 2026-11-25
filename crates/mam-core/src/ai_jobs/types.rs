//! 단계 종류·상태, 주입 trait 과 요청/결과 구조체. `mam-ai` 에 의존하지 않는다 -
//! 백엔드 소유자가 `mam_ai::external::{ExternalClient, AnalyzeRequest, ...}` 위에 얇은 어댑터를 구현한다.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ai_store::TranscriptRecord;

/// 분석 단계 종류 = UI `AnalysisKind` = `ai_task_state.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Describe,
    Keywords,
    Ocr,
    Summary,
    Stt,
    Embedding,
}

impl StepKind {
    pub const ALL: [StepKind; 6] = [
        Self::Describe,
        Self::Keywords,
        Self::Ocr,
        Self::Summary,
        Self::Stt,
        Self::Embedding,
    ];
    /// 외부 AI 로 요청하는 종류 (동의 필요)
    pub const EXTERNAL: [StepKind; 4] = [Self::Describe, Self::Keywords, Self::Ocr, Self::Summary];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Describe => "describe",
            Self::Keywords => "keywords",
            Self::Ocr => "ocr",
            Self::Summary => "summary",
            Self::Stt => "stt",
            Self::Embedding => "embedding",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
    pub fn is_external(self) -> bool {
        Self::EXTERNAL.contains(&self)
    }
    /// 이 미디어 유형(`image`/`video`/`audio`)에 적용되는 단계인가.
    /// 음성은 원본을 외부로 보내지 않으므로(정책 미정) 로컬 단계(stt·embedding)만 적용한다.
    pub fn applies_to(self, media_type: &str) -> bool {
        match (self, media_type) {
            (Self::Stt, "image") => false,
            (k, "audio") => !k.is_external(),
            (_, "image" | "video") => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Queued,
    Running,
    Done,
    Failed,
    Skipped,
}

impl StepState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// UI 계약 `AnalysisStatus` (`state` 는 행이 없으면 `none`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepStatus {
    pub kind: StepKind,
    pub state: String,
    pub error_code: Option<String>,
    pub updated_at: Option<i64>,
}

/// 단계 실패. `message` 는 **정제된 사용자 메시지**여야 한다 (공급자 응답 본문·키·경로 금지) - 이력에 그대로 저장된다.
/// `code` 분류는 [`classify`](super::classify): 재시도 `offline|timeout|rate_limited|server_error|source_unavailable|
/// thumbnail_missing|io_transient`, 건너뜀 `consent_required|model_missing|disabled|unsupported`,
/// 중단 `cancelled`, 그 밖에는 영구 실패(`auth_failed|bad_request|bad_response|invalid_input|stt_timeout|...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepFailure {
    pub code: String,
    pub message: String,
    /// 429 `Retry-After` (초)
    pub retry_after_secs: Option<u64>,
    /// 실제로 과금된 인증 방식 (`apiKey` | `subscription`) — 알 때만. 구독 모드인데 CLI 가 환경변수 API 키를 쓴 경우 등
    pub billed_auth_mode: Option<String>,
}

impl StepFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retry_after_secs: None,
            billed_auth_mode: None,
        }
    }
    pub fn with_billed_auth_mode(mut self, mode: Option<String>) -> Self {
        self.billed_auth_mode = mode;
        self
    }
    pub fn with_retry_after(mut self, secs: Option<u64>) -> Self {
        self.retry_after_secs = secs;
        self
    }
}

// ── 외부 AI (mam_ai::external 미러) ───────────────────────────────────────

/// 외부 AI 로 보낼 설정 (UI `TransferScope`). 어댑터가 `TransferScope::clamped` 로 보정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    pub image_max_edge: u32,
    pub video_frames: u8,
    pub include_metadata: bool,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            image_max_edge: 1024,
            video_frames: 5,
            include_metadata: true,
        }
    }
}

/// 프레임 스트립(`cache/previews/.../<id>_strip.webp`, 같은 크기의 칸 `cells` 개를 가로로 이은 WebP 한 장)의 한 칸.
/// 칸 `index` 의 폭은 `전체 폭 / cells`, 왼쪽부터 `index * 칸폭`. `index` 는 영상 안의 위치 순서(10%, 30%, ...)이며
/// 벡터의 `frame_index` 로 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameCell {
    pub index: u32,
    pub cells: u32,
}

/// 분석·임베딩에 쓸 이미지 한 장. 항상 MAMData `cache/` 안의 파일이다 - 원본 경로는 없다.
/// `frame` 이 `Some` 이면 `path` 는 프레임 스트립이고 어댑터가 그 칸만 잘라 쓴다 (영상). `None` 이면 파일 전체가 한 장(썸네일).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub path: PathBuf,
    pub frame: Option<FrameCell>,
}

impl ImageInput {
    /// 벡터 `frame_index` (프레임이면 `VectorSource::Frame`, 아니면 `Image`).
    pub fn frame_index(&self) -> Option<i64> {
        self.frame.map(|f| i64::from(f.index))
    }
}

/// 외부 분석 요청. 이미지는 MAMData `cache/` 의 썸네일 또는 영상 프레임 스트립의 칸뿐이다 - 원본 경로는 없다.
/// 영상은 스트립이 있으면 최대 `transfer.video_frames` 칸(고르게 선택), 스트립을 만들 수 없는 영상은 썸네일 1장.
/// 어댑터가 파일을 읽어 축소·인코딩하고 `SanitizedMeta`(파일명·촬영일·유형·길이 + 사용자 메타데이터 필드 `(이름, 표시 값)`, 절대 경로 제외)를 만든다.
/// 사용자 필드는 `transfer.include_metadata` 가 켜져 있을 때만 읽고 보낸다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalRequest {
    pub asset_id: i64,
    /// `image` | `video`
    pub media_type: String,
    pub kinds: Vec<StepKind>,
    pub language: String,
    pub transfer: Transfer,
    pub images: Vec<ImageInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_bytes: u64,
    pub images: u32,
    /// 실제 과금된 인증 방식 (`apiKey` | `subscription`). None = 어댑터가 모른다 -> `ExternalAnalyzer::auth_mode()` 로 대체
    pub auth_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalResult {
    pub description: String,
    pub keywords: Vec<String>,
    pub ocr_text: String,
    pub summary: String,
    pub language: String,
    /// openai | gemini | claude
    pub provider: String,
    pub model: String,
    pub usage: ExternalUsage,
}

/// 외부 AI 어댑터 (`ExternalClient` 를 감싼다). 어댑터는 `UsageSink` 로 사용량을 **따로 저장하지 않는다** -
/// 실행기가 `ai_usage` 에 기록한다 (이중 기록 방지). 재시도 루프도 어댑터에 두지 않는다 (대기열 백오프가 담당).
pub trait ExternalAnalyzer: Send + Sync {
    /// 현재 선택된 공급자 이름 (실패 사용량·이력 기록용). 설정 안 됐으면 `"none"`.
    fn provider(&self) -> String;
    /// 현재 설정된 인증 방식 (`apiKey` | `subscription`) - 실패 사용량 기록용. 모르면 None (NULL 로 저장).
    fn auth_mode(&self) -> Option<String> {
        None
    }
    /// 구독 한도 등으로 외부 요청을 보내면 안 되는 시각 (유닉스 ms). 지금이 그 전이면 실행기는 요청을 만들지 않고
    /// 시도 횟수를 올리지 않은 채 작업을 그 시각까지 미룬다. 기본은 None (제한 없음).
    fn paused_until_ms(&self) -> Option<i64> {
        None
    }
    fn analyze(&self, req: &ExternalRequest) -> Result<ExternalResult, StepFailure>;
}

// ── 로컬 STT ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttRequest {
    pub asset_id: i64,
    /// 원본 미디어 (읽기 전용)
    pub media_path: PathBuf,
    pub duration_ms: Option<u64>,
    /// 자산별 제한 시간 (`AiJobsConfig::stt_timeout`) - 넘으면 `stt_timeout` 로 실패시킨다
    pub timeout: Duration,
}

/// 로컬 STT 어댑터 (`mam_ai::local::stt::transcribe_asset` 을 감싼다). `cancel` 이 true 가 되면 `cancelled` 로 끝낸다.
pub trait SttRunner: Send + Sync {
    fn transcribe(
        &self,
        req: &SttRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<TranscriptRecord, StepFailure>;
}

// ── 임베딩 ───────────────────────────────────────────────────────────────

/// 임베딩 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedMode {
    /// 시각 + 의미
    Full,
    /// 의미 벡터만 다시 만든다 - 사용자 편집·전사 갱신 뒤. `images` 는 비어 있고 시각 벡터는 건드리지 않는다.
    SemanticOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedRequest {
    pub asset_id: i64,
    pub mode: EmbedMode,
    /// MAMData 안의 썸네일·프레임 스트립 칸 (음성·`SemanticOnly` 는 빈 목록) - 시각 벡터용.
    /// 프레임(`frame` 이 `Some`)마다 `VectorSource::Frame` + `frame_index` 벡터 하나, 썸네일은 `Image` 벡터 하나.
    pub images: Vec<ImageInput>,
    /// AI 설명·키워드·OCR·요약·전사 (의미 벡터용, 비어 있을 수 있다)
    pub text_parts: Vec<String>,
}

/// 임베딩 어댑터 (`embedding::embed_visual` / `embed_semantic` 을 감싼다). 모델 미준비는 `model_missing`.
/// `SemanticOnly` 에서는 `embed_semantic` 만 부른다.
pub trait EmbeddingRunner: Send + Sync {
    fn embed(&self, req: &EmbedRequest) -> Result<(), StepFailure>;
}
