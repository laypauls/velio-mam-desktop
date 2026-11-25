//! `--output-format stream-json` 해석 — 한 줄씩 먹여 상태를 쌓는다. 모르는 줄·필드는 무시한다(CLI 업그레이드 내성).
//!
//! 쓰는 줄: `system/init`(`apiKeySource`, `model`), `system/api_retry`(`error_status`), `rate_limit_event`
//! (`rate_limit_info.status`, `resetsAt`), `result`(`is_error`, `result`, `structured_output`, `usage`, `api_error_status`).
//! 응답 내용·CLI 오류 문구는 사용자 메시지·로그로 내보내지 않는다(분류에만 쓴다).

use serde_json::Value;

use super::exec::Flow;
use crate::external::error::ExternalError;

/// `error_status: null` 인 `api_retry` 가 이만큼 연속되면 오프라인으로 보고 중단한다
/// (실측: 죽은 프록시에서 CLI 는 170초 동안 10회 재시도한 뒤에야 실패했다).
pub const OFFLINE_AFTER_RETRIES: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliResult {
    pub is_error: bool,
    pub text: String,
    pub structured: Option<Value>,
    pub http_status: Option<i64>,
    pub api_error: Option<String>,
    /// 입력 토큰 총합 (일반 + 캐시 생성 + 캐시 읽기)
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Default)]
pub struct StreamState {
    /// `system/init.apiKeySource` — `"none"` 이면 구독 로그인, 그 밖이면 API 키(환경변수)로 과금된다
    pub api_key_source: Option<String>,
    /// CLI 가 실제로 쓴 모델 ID
    pub model: Option<String>,
    pub result: Option<CliResult>,
    /// 한도 거부 이벤트를 봤다
    pub limit_rejected: bool,
    /// 한도 초기화 시각(Unix 초)
    pub limit_resets_at: Option<i64>,
    /// 오프라인으로 조기 중단했다
    pub offline: bool,
    null_retries: u32,
}

impl StreamState {
    /// 과금이 API 키로 되는가 (init 을 못 본 경우는 모르므로 false).
    pub fn billed_to_api_key(&self) -> bool {
        self.api_key_source.as_deref().is_some_and(|s| s != "none")
    }

    pub fn feed(&mut self, line: &str) -> Flow {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return Flow::Continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("system") => self.system(&v),
            Some("rate_limit_event") => self.rate_limit(&v),
            Some("assistant") => self.null_retries = 0,
            Some("result") => self.result = Some(parse_result(&v)),
            _ => {}
        }
        if self.null_retries >= OFFLINE_AFTER_RETRIES && self.result.is_none() {
            self.offline = true;
            return Flow::Stop;
        }
        Flow::Continue
    }

    fn system(&mut self, v: &Value) {
        match v.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                self.api_key_source = v
                    .get("apiKeySource")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.model = v.get("model").and_then(Value::as_str).map(str::to_string);
            }
            Some("api_retry") => {
                if v.get("error_status").is_none_or(Value::is_null) {
                    self.null_retries += 1;
                } else {
                    self.null_retries = 0;
                }
            }
            _ => {}
        }
    }

    fn rate_limit(&mut self, v: &Value) {
        let Some(info) = v.get("rate_limit_info") else {
            return;
        };
        if info.get("status").and_then(Value::as_str) == Some("rejected") {
            self.limit_rejected = true;
            self.limit_resets_at = info.get("resetsAt").and_then(Value::as_i64);
        }
    }
}

fn parse_result(v: &Value) -> CliResult {
    let subtype_ok = v
        .get("subtype")
        .and_then(Value::as_str)
        .is_none_or(|s| s == "success");
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false) || !subtype_ok;
    let usage = v.get("usage");
    let n = |k: &str| {
        usage
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    CliResult {
        is_error,
        text: v
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        structured: v.get("structured_output").filter(|s| !s.is_null()).cloned(),
        http_status: v.get("api_error_status").and_then(Value::as_i64),
        api_error: v
            .get("api_error")
            .and_then(Value::as_str)
            .map(str::to_string),
        input_tokens: n("input_tokens")
            .saturating_add(n("cache_creation_input_tokens"))
            .saturating_add(n("cache_read_input_tokens")),
        output_tokens: n("output_tokens"),
    }
}

/// `is_error` 결과 -> 오류. `retry_after_secs` 는 한도 거부 시 호출자가 시계로 계산한 값.
pub fn map_error_result(
    r: &CliResult,
    st: &StreamState,
    retry_after_secs: Option<u64>,
) -> ExternalError {
    let text = r.text.to_ascii_lowercase();
    let not_logged_in = text.contains("not logged in") || text.contains("/login");
    match r.http_status {
        Some(401 | 403) => {
            // 환경변수 API 키가 쓰인 경우의 401 은 로그인 문제가 아니라 그 키의 문제다
            if st.billed_to_api_key() {
                ExternalError::AuthFailed
            } else {
                ExternalError::CliNotLoggedIn
            }
        }
        // 환경변수 API 키로 과금된 요청의 429 는 구독 한도가 아니라 그 키의 속도 제한이다 (구독을 멈추면 안 된다)
        Some(429) if st.billed_to_api_key() => ExternalError::RateLimited { retry_after_secs },
        Some(429) => ExternalError::SubscriptionLimit { retry_after_secs },
        _ if not_logged_in => ExternalError::CliNotLoggedIn,
        _ if st.limit_rejected
            || text.contains("usage limit")
            || text.contains("limit reached") =>
        {
            ExternalError::SubscriptionLimit { retry_after_secs }
        }
        Some(s) if (500..600).contains(&s) => ExternalError::Server { status: s as u16 },
        Some(s) if (400..500).contains(&s) => ExternalError::BadRequest { status: s as u16 },
        None if is_connection_failure(r, &text) => ExternalError::Offline,
        _ => ExternalError::BadResponse("cli reported an error"),
    }
}

fn is_connection_failure(r: &CliResult, lower_text: &str) -> bool {
    r.api_error.as_deref() == Some("connection_failed")
        || lower_text.contains("econn")
        || lower_text.contains("connection refused")
        || lower_text.contains("enotfound")
        || lower_text.contains("timed out")
}

/// stderr 만으로 로그인 문제를 가려낸다 (결과 줄 없이 종료한 경우).
pub fn stderr_says_not_logged_in(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("not logged in") || s.contains("/login") || s.contains("auth login")
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
