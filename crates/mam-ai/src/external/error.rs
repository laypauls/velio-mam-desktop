//! 외부 AI 오류. `Display`/`Debug` 에는 API 키·응답 본문·요청 헤더를 넣지 않는다 (상태 코드와 고정 문구만).

/// 내부 세분 오류. UI 로는 [`ExternalError::ui_code`] 의 코드 둘로만 나간다 (PL-A7).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExternalError {
    #[error("external AI consent is required")]
    ConsentRequired,
    /// 401/403 이거나 키가 없다.
    #[error("authentication failed")]
    AuthFailed,
    #[error("rate limited")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("offline")]
    Offline,
    #[error("request timed out")]
    Timeout,
    #[error("provider rejected the request (HTTP {status})")]
    BadRequest { status: u16 },
    #[error("provider server error (HTTP {status})")]
    Server { status: u16 },
    /// 응답 형식이 기대와 다르다. 이유는 고정 문구(응답 내용 아님).
    #[error("unusable provider response: {0}")]
    BadResponse(&'static str),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// 구독 모드: Claude Code CLI 를 찾을 수 없다.
    #[error("CLI not found")]
    CliNotFound,
    /// 구독 모드: CLI 버전이 최소 지원 버전보다 낮거나 확인할 수 없다.
    #[error("CLI version unsupported")]
    CliUnsupportedVersion,
    /// 구독 모드: CLI 가 로그인되어 있지 않다.
    #[error("CLI is not logged in")]
    CliNotLoggedIn,
    /// 구독 사용 한도 거부. `retry_after_secs` 는 한도 초기화까지 남은 시간(상한 보정됨).
    #[error("subscription usage limit reached")]
    SubscriptionLimit { retry_after_secs: Option<u64> },
    /// 종료·취소로 중단했다 (요청은 끝까지 가지 않았을 수 있다).
    #[error("cancelled")]
    Cancelled,
}

impl ExternalError {
    /// 작업 이력·UI `errorCode` 에 쓰는 내부 코드.
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::ConsentRequired => "consent_required",
            Self::AuthFailed => "auth_failed",
            Self::RateLimited { .. } => "rate_limited",
            Self::Offline => "offline",
            Self::Timeout => "timeout",
            Self::BadRequest { .. } => "bad_request",
            Self::Server { .. } => "server_error",
            Self::BadResponse(_) => "bad_response",
            Self::InvalidInput(_) => "invalid_input",
            Self::CliNotFound => "cli_not_found",
            Self::CliUnsupportedVersion => "cli_unsupported_version",
            Self::CliNotLoggedIn => "cli_not_logged_in",
            Self::SubscriptionLimit { .. } => "subscription_limit",
            Self::Cancelled => "cancelled",
        }
    }

    /// UI 로 노출하는 오류 코드: `consent_required` | `provider_error` | `invalid_input`.
    pub fn ui_code(&self) -> &'static str {
        match self {
            Self::ConsentRequired => "consent_required",
            Self::InvalidInput(_) => "invalid_input",
            _ => "provider_error",
        }
    }

    /// 사용자에게 보여도 되는 정제된 한국어 메시지.
    pub fn user_message(&self) -> String {
        match self {
            Self::ConsentRequired => "외부 AI 전송에 대한 동의가 필요합니다".into(),
            Self::AuthFailed => "API 키를 확인하세요".into(),
            Self::RateLimited { .. } => {
                "요청 한도에 도달했습니다. 잠시 후 자동으로 다시 시도합니다".into()
            }
            Self::Offline => {
                "네트워크에 연결할 수 없어 외부 AI 분석만 중단됩니다. 관리·검색 기능은 계속 사용할 수 있습니다"
                    .into()
            }
            Self::Timeout => "외부 AI 응답 시간이 초과되었습니다".into(),
            Self::BadRequest { status } => {
                format!("외부 AI 가 요청을 거부했습니다 (HTTP {status})")
            }
            Self::Server { status } => {
                format!("외부 AI 서버 오류가 발생했습니다 (HTTP {status})")
            }
            Self::BadResponse(_) => "외부 AI 응답을 해석할 수 없습니다".into(),
            Self::InvalidInput(m) => format!("잘못된 입력입니다: {m}"),
            Self::CliNotFound => "Claude Code CLI 를 찾을 수 없습니다. 설치 후 다시 확인하세요".into(),
            Self::CliUnsupportedVersion => {
                "Claude Code CLI 버전이 지원 범위보다 낮습니다. 업데이트하세요".into()
            }
            Self::CliNotLoggedIn => "Claude 구독 계정에 로그인되어 있지 않습니다".into(),
            Self::SubscriptionLimit { .. } => {
                "구독 사용 한도에 도달했습니다. 한도가 초기화된 뒤 다시 시도합니다".into()
            }
            Self::Cancelled => "작업이 중단되었습니다".into(),
        }
    }

    /// UI `ProviderCheck.errorCode`: auth_failed | offline | rate_limited | quota_exceeded | unknown.
    /// (429 와 할당량 초과는 응답 본문 없이 구분할 수 없어 `rate_limited` 로 통일한다.)
    pub fn check_code(&self) -> &'static str {
        match self {
            Self::AuthFailed => "auth_failed",
            Self::Offline | Self::Timeout => "offline",
            Self::RateLimited { .. } => "rate_limited",
            Self::CliNotFound => "cli_not_found",
            Self::CliUnsupportedVersion => "cli_unsupported_version",
            Self::CliNotLoggedIn => "cli_not_logged_in",
            Self::SubscriptionLimit { .. } => "subscription_limit",
            _ => "unknown",
        }
    }
}
