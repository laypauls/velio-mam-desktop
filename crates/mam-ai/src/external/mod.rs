//! 외부 AI 코어 (P3-2) — OpenAI·Gemini·Claude 공식 REST 어댑터.
//!
//! 규칙: 동의 전 네트워크 호출 0 (`ConsentGate`), 전송은 MAM 파생 이미지 + 정제 메타뿐, 키·응답 본문은 오류에 없음,
//! 429/5xx/timeout 재시도, 요청마다 사용량 기록. HTTP 는 [`HttpTransport`] 로 주입한다(테스트는 스텁, 실호출 금지).
//! 모델 ID 기본값은 [`models`] 한 곳 — 릴리스 전 검토 필요.

mod adapter;
mod claude;
pub mod cli;
mod client;
mod consent;
mod error;
mod gemini;
pub mod models;
mod openai;
mod payload;
mod prompt;
mod retry;
mod transport;
mod types;
#[cfg(test)]
mod umeta_tests;

pub use adapter::REQUEST_TIMEOUT;
pub use client::{AiClient, ExternalClient, check_provider};
pub use consent::ConsentGate;
pub use error::ExternalError;
pub use payload::{AnalyzeRequest, MediaType, PreparedImage, SanitizedMeta, prepare_image};
pub use retry::{Clock, RetryPolicy, Sleeper, SystemClock, ThreadSleeper, parse_retry_after};
pub use transport::{HttpRequest, HttpResponse, HttpTransport, TransportError, UreqTransport};
pub use types::{
    AnalysisKind, AnalyzeOutput, AuthMode, Provider, ProviderCheck, TransferScope, UsageRecord,
    UsageSink,
};
