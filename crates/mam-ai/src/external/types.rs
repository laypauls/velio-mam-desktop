//! 외부 AI 공용 값 타입 — 공급자·분석 종류·전송 범위·결과·사용량. 공개 결과 타입은 UI 계약(camelCase)과 맞춘다.

use serde::{Deserialize, Serialize};

/// 외부 AI 공급자. 직렬화 값은 UI 의 `AiProvider` 와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "claude")]
    Claude,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::OpenAi, Provider::Gemini, Provider::Claude];

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Gemini => "gemini",
            Provider::Claude => "claude",
        }
    }
}

/// 공급자 인증 방식. `apiKey` = 공식 API 키(기존), `subscription` = 사용자가 설치한 Claude Code CLI 의 구독 로그인(claude 만).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AuthMode {
    #[default]
    #[serde(rename = "apiKey")]
    ApiKey,
    #[serde(rename = "subscription")]
    Subscription,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::ApiKey => "apiKey",
            AuthMode::Subscription => "subscription",
        }
    }
}

/// 외부로 요청할 수 있는 분석 종류 (STT·임베딩은 로컬 전용이라 없다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisKind {
    Describe,
    Keywords,
    Ocr,
    Summary,
}

impl AnalysisKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AnalysisKind::Describe => "describe",
            AnalysisKind::Keywords => "keywords",
            AnalysisKind::Ocr => "ocr",
            AnalysisKind::Summary => "summary",
        }
    }
}

pub const IMAGE_EDGE_RANGE: (u32, u32) = (256, 2048);
pub const VIDEO_FRAMES_RANGE: (u8, u8) = (1, 10);

/// UI `TransferScope` — 외부로 보내는 범위. 범위를 벗어난 값은 [`TransferScope::clamped`] 로 보정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferScope {
    pub image_max_edge: u32,
    pub video_frames: u8,
    pub include_metadata: bool,
}

impl Default for TransferScope {
    fn default() -> Self {
        Self {
            image_max_edge: 1024,
            video_frames: 5,
            include_metadata: true,
        }
    }
}

impl TransferScope {
    pub fn clamped(self) -> Self {
        Self {
            image_max_edge: self
                .image_max_edge
                .clamp(IMAGE_EDGE_RANGE.0, IMAGE_EDGE_RANGE.1),
            video_frames: self
                .video_frames
                .clamp(VIDEO_FRAMES_RANGE.0, VIDEO_FRAMES_RANGE.1),
            include_metadata: self.include_metadata,
        }
    }
}

/// 요청 1건의 사용량 기록 (PL-A5). 저장은 호출자(SQLite)가 한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub provider: Provider,
    pub model: String,
    /// 한 요청에 담은 분석 종류를 `+` 로 이은 값 (예: `describe+keywords+ocr`), 연결 확인은 `check`.
    pub kind: String,
    pub asset_id: Option<i64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_bytes: u64,
    pub ok: bool,
    /// 실패 시 내부 오류 코드 (`auth_failed`, `offline` ...).
    pub error_code: Option<String>,
    /// Unix ms.
    pub at: i64,
    /// 실제로 과금된 인증 방식. 구독 모드여도 CLI 가 환경변수 API 키를 쓰면 `ApiKey` 로 기록한다.
    #[serde(default)]
    pub auth_mode: AuthMode,
}

/// 분석 결과 — UI `AiAnalysis` 의 모델 생성 부분 (`provider`/`model`/`analyzedAt` 은 `usage` 에서 얻는다).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeOutput {
    pub description: String,
    pub keywords: Vec<String>,
    pub ocr_text: String,
    pub summary: String,
    pub language: String,
    pub provider: Provider,
    pub model: String,
    pub usage: UsageRecord,
}

/// UI `ProviderCheck` — 연결 확인 결과. `error_code`: auth_failed | offline | rate_limited | quota_exceeded | unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheck {
    pub ok: bool,
    pub error_code: Option<String>,
}

/// 요청마다 호출되는 사용량 수신자 (성공·실패 모두). 호출자가 SQLite 기록에 연결한다.
pub trait UsageSink: Send + Sync {
    fn record(&self, usage: &UsageRecord);
}
