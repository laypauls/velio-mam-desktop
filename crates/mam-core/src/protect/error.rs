//! 보호 계층의 타입화된 오류와 UI 계약 결과 타입.
//!
//! 오류 메시지·Debug 출력에는 비밀번호나 해시 문자열을 넣지 않는다.

use serde::Serialize;

/// 비즈니스 규칙 위반과 외부 실패. [`ProtectError::code`] 가 UI 오류 코드(snake_case)를 돌려준다.
#[derive(Debug, thiserror::Error)]
pub enum ProtectError {
    /// 입력 검증 실패 (비밀번호 길이, 빈 자산 목록, 잘못된 대상 폴더 등)
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("password is not set")]
    PasswordNotSet,
    #[error("password is already set")]
    PasswordAlreadySet,
    #[error("password is incorrect")]
    PasswordIncorrect,
    /// 인증 면제 시간 밖에서 보호 명령을 호출했다
    #[error("authentication required")]
    AuthRequired,
    /// 연속 실패 제한 (D2) — `retry_after_ms` 뒤에 다시 시도할 수 있다
    #[error("too many failed attempts; retry after {retry_after_ms} ms")]
    TooManyAttempts { retry_after_ms: i64 },
    #[error("not found: {0}")]
    NotFound(String),
    /// 볼륨 오프라인 또는 파일 없음 — 자산별 실패로 보고한다
    #[error("offline or missing: {0}")]
    Offline(String),
    #[error("I/O error: {0}")]
    Io(String),
    /// 초기화 전 백업이 없거나 실패했다 — 초기화는 진행하지 않는다
    #[error("backup required before reset: {0}")]
    BackupFailed(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ProtectError {
    /// UI 오류 코드. `too_many_attempts` 는 계약 목록 밖의 추가 코드다.
    /// `BackupFailed` 는 `io` 로 보고한다.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::PasswordNotSet => "password_not_set",
            Self::PasswordAlreadySet => "password_already_set",
            Self::PasswordIncorrect => "password_incorrect",
            Self::AuthRequired => "auth_required",
            Self::TooManyAttempts { .. } => "too_many_attempts",
            Self::NotFound(_) => "not_found",
            Self::Offline(_) => "offline",
            Self::Io(_) | Self::BackupFailed(_) => "io",
            Self::Internal(_) => "internal",
        }
    }

    pub(crate) fn io(context: &std::path::Path, e: &std::io::Error) -> Self {
        Self::Io(format!("{}: {e}", context.display()))
    }
}

impl From<rusqlite::Error> for ProtectError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(format!("database error: {e}"))
    }
}

impl From<crate::CoreError> for ProtectError {
    fn from(e: crate::CoreError) -> Self {
        match e {
            crate::CoreError::Io { .. } => Self::Io(e.to_string()),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// 자산별 실패 한 건 (UI `OriginalsResult.failed[]`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFailure {
    pub asset_id: i64,
    pub message: String,
}

/// UI 계약 `OriginalsResult` — 성공 개수와 자산별 실패 목록
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginalsResult {
    pub succeeded: u32,
    pub failed: Vec<AssetFailure>,
}

/// UI 계약 `ProtectionStatus`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionStatus {
    pub password_set: bool,
    /// 인증 면제 만료 시각(UTC ms). `None` = 인증 안 됨
    pub auth_valid_until: Option<i64>,
}

/// 자산별 실패 메시지로 허용하는 고정 코드. 경로·DB 오류 원문은 넣지 않는다 (상세는 `tracing` 로그에만).
const ASSET_CODES: [&str; 14] = [
    "file_missing",
    "offline",
    "changed_since_scan",
    "destination_exists",
    "destination_tracked",
    "already_in_destination",
    "recycle_failed",
    "no_recycle_bin",
    "moved_db_pending",
    "copy_verify_failed",
    "invalid_input",
    "not_found",
    "internal",
    "io",
];

impl ProtectError {
    /// `OriginalsResult.failed[].message` 용 고정 코드 문자열 (경로·원문 없음).
    /// 내부 오류는 자신의 문자열이 허용 코드일 때만 그 코드를, 아니면 변형별 일반 코드를 쓴다.
    pub fn asset_message(&self) -> &'static str {
        let inner = match self {
            Self::Offline(s)
            | Self::Io(s)
            | Self::InvalidInput(s)
            | Self::NotFound(s)
            | Self::Internal(s) => s.as_str(),
            _ => "",
        };
        if let Some(code) = ASSET_CODES.iter().find(|c| **c == inner) {
            return code;
        }
        match self {
            Self::Offline(_) => "offline",
            Self::Io(_) | Self::BackupFailed(_) => "io",
            Self::NotFound(_) => "not_found",
            Self::InvalidInput(_) => "invalid_input",
            _ => "internal",
        }
    }
}
