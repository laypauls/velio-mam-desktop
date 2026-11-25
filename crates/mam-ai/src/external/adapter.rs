//! 공급자 어댑터 공통 계약 — 요청 형식 생성과 응답 해석만 담당하고 네트워크는 건드리지 않는다.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

use super::error::ExternalError;
use super::payload::PreparedImage;
use super::transport::HttpRequest;
use super::types::Provider;

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 공급자 응답에서 뽑은 모델 텍스트와 토큰 사용량.
#[derive(Debug)]
pub(crate) struct Reply {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub(crate) trait Adapter: Send + Sync {
    fn provider(&self) -> Provider;
    /// `key` 는 헤더에만 넣는다 — URL·본문에 넣지 않는다.
    fn build(
        &self,
        key: &str,
        model: &str,
        prompt: &str,
        images: &[PreparedImage],
        max_tokens: u32,
    ) -> HttpRequest;
    fn parse(&self, body: &[u8]) -> Result<Reply, ExternalError>;
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

pub(crate) fn json_request(url: String, headers: Vec<(&str, String)>, body: &Value) -> HttpRequest {
    let mut h: Vec<(String, String)> = vec![("content-type".into(), "application/json".into())];
    h.extend(headers.into_iter().map(|(k, v)| (k.to_string(), v)));
    HttpRequest {
        method: "POST",
        url,
        headers: h,
        body: serde_json::to_vec(body).unwrap_or_default(),
        timeout: REQUEST_TIMEOUT,
    }
}

pub(crate) fn parse_json(body: &[u8]) -> Result<Value, ExternalError> {
    serde_json::from_slice(body).map_err(|_| ExternalError::BadResponse("response is not JSON"))
}

pub(crate) fn tokens(v: Option<&Value>) -> u64 {
    v.and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn no_text() -> ExternalError {
    ExternalError::BadResponse("response has no text")
}
