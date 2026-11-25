//! 단계 실패 분류 (D3) - 오류 코드 문자열 -> 재시도 / 건너뜀 / 중단 / 영구 실패.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// 일시 오류 - 대기열 백오프로 다시 시도
    Retry,
    /// 이 환경에서는 할 수 없다 (동의 없음·모델 없음·비활성) - `skipped`, 실패 아님
    Skip,
    /// 일시정지·종료로 중단 - 시도 횟수를 올리지 않고 양보
    Cancel,
    /// 같은 입력으로 반복해도 소용없다 - `failed`
    Permanent,
    /// 계정 단위 한도(구독 사용량 한도) - 시도 횟수를 올리지 않고 한도가 풀릴 때까지 작업을 미룬다
    Pause,
}

pub fn classify(code: &str) -> FailureClass {
    match code {
        "offline" | "timeout" | "rate_limited" | "server_error" | "source_unavailable"
        | "thumbnail_missing" | "io_transient" => FailureClass::Retry,
        "consent_required" | "model_missing" | "disabled" | "unsupported" | "asset_inactive" => {
            FailureClass::Skip
        }
        "cancelled" => FailureClass::Cancel,
        "subscription_limit" => FailureClass::Pause,
        _ => FailureClass::Permanent,
    }
}

/// 이 실패가 공급자까지 요청이 나간 뒤의 것인가 (사용량 행 기록 기준).
/// CLI 가 없거나 버전이 낮은 경우는 프로세스를 띄우기 전에 끝나므로 요청이 없다.
pub fn request_was_sent(code: &str) -> bool {
    !matches!(
        code,
        "cli_not_found" | "cli_unsupported_version" | "cli_not_logged_in"
    )
}

/// 한 번의 실행에서 일어난 재시도 가능 실패 요약 -> `JobError` 로 변환한다.
#[derive(Debug, Default)]
pub struct RetryNote {
    pub codes: Vec<String>,
    pub retry_after: Option<Duration>,
}

impl RetryNote {
    pub fn add(&mut self, code: &str, retry_after_secs: Option<u64>) {
        if !self.codes.iter().any(|c| c == code) {
            self.codes.push(code.to_string());
        }
        if let Some(s) = retry_after_secs {
            let d = Duration::from_secs(s);
            self.retry_after = Some(self.retry_after.map_or(d, |old| old.max(d)));
        }
    }
    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }
    /// 메시지는 오류 코드만 담는다 (본문·키 없음).
    pub fn into_job_error(self) -> crate::jobs::JobError {
        let msg = format!("ai steps will retry: {}", self.codes.join(","));
        match self.retry_after {
            Some(d) => crate::jobs::JobError::retryable_after(msg, d),
            None => crate::jobs::JobError::retryable(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Business rule
    #[test]
    fn t_aij_fail_02_pre_spawn_cli_failures_did_not_send_a_request() {
        for code in [
            "cli_not_found",
            "cli_unsupported_version",
            "cli_not_logged_in",
        ] {
            assert!(!request_was_sent(code), "{code}");
        }
        for code in ["subscription_limit", "offline", "bad_response", "timeout"] {
            assert!(request_was_sent(code), "{code}");
        }
    }

    #[test]
    fn t_aij_fail_01_subscription_codes_are_classified() {
        assert_eq!(classify("subscription_limit"), FailureClass::Pause);
        assert_eq!(classify("cancelled"), FailureClass::Cancel);
        // 사용자가 직접 고쳐야 하는 구독 CLI 문제는 재시도하지 않고 실패로 남겨 보여 준다
        for code in [
            "cli_not_found",
            "cli_unsupported_version",
            "cli_not_logged_in",
        ] {
            assert_eq!(classify(code), FailureClass::Permanent, "{code}");
        }
    }
}
