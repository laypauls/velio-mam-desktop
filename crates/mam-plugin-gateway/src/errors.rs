//! 게이트웨이 오류 — 코드는 `mam_plugin_host::codes`, 메시지는 아래 정적 표에서만 나온다(입력·경로·SQL 을 되돌려 보내지 않는다).
//! `Internal` 의 상세는 호출한 쪽이 `tracing::error` 로만 남긴다.

use mam_db::PoolError;
use mam_plugin_host::{HostError, codes};
use mam_plugin_manifest::Permission;

#[derive(Debug)]
pub(crate) enum GwError {
    /// 필요한 공개 권한 문자열(메시지에 허용)
    PermissionDenied(Permission),
    BadRequest(&'static str),
    UnknownMethod,
    NotFound,
    TooLarge(&'static str),
    Quota(&'static str),
    Unavailable,
    Busy,
    /// 상세(로그 전용)
    Internal(String),
}

/// 사용자(플러그인)에게 나갈 수 있는 모든 메시지 — 누출 시험이 이 표와 대조한다
pub const ALLOWED_MESSAGES: &[&str] = &[
    "permission denied: asset.read.metadata",
    "permission denied: asset.read.derived",
    "permission denied: kv.read",
    "permission denied: kv.write",
    "permission denied: tag.write",
    "permission denied: export.write",
    "invalid params",
    "invalid asset id",
    "invalid key",
    "invalid prefix",
    "invalid cursor",
    "invalid limit",
    "invalid tag name",
    "invalid tag list",
    "invalid derived kind",
    "unknown method",
    "asset not found",
    "value too large",
    "derived data too large",
    "kv quota exceeded",
    "asset tag limit reached",
    "new tag budget exceeded",
    "derived data budget exceeded",
    "derived data is not available",
    "host is busy",
    "host function failed",
];

impl GwError {
    pub(crate) fn internal(detail: impl ToString) -> Self {
        Self::Internal(detail.to_string())
    }

    /// 코드·정적 메시지로 변환한다. `Internal` 은 상세를 로그로만 남긴다
    pub(crate) fn into_host(self, plugin: &str, method: &str) -> HostError {
        let (code, msg): (&str, &str) = match self {
            Self::PermissionDenied(p) => (codes::PERMISSION_DENIED, permission_message(p)),
            Self::BadRequest(m) => (codes::BAD_REQUEST, m),
            Self::UnknownMethod => (codes::UNKNOWN_METHOD, "unknown method"),
            Self::NotFound => (codes::NOT_FOUND, "asset not found"),
            Self::TooLarge(m) => (codes::TOO_LARGE, m),
            Self::Quota(m) => (codes::QUOTA_EXCEEDED, m),
            Self::Unavailable => (codes::UNAVAILABLE, "derived data is not available"),
            Self::Busy => (codes::BUSY, "host is busy"),
            Self::Internal(detail) => {
                tracing::error!(target: "mam::plugin", plugin_id = plugin, method = method, error = %detail, "host function failed");
                (codes::INTERNAL, "host function failed")
            }
        };
        HostError::new(code, msg)
    }
}

fn permission_message(perm: Permission) -> &'static str {
    match perm {
        Permission::AssetReadMetadata => "permission denied: asset.read.metadata",
        Permission::AssetReadDerived => "permission denied: asset.read.derived",
        Permission::KvRead => "permission denied: kv.read",
        Permission::KvWrite => "permission denied: kv.write",
        Permission::TagWrite => "permission denied: tag.write",
        Permission::ExportWrite => "permission denied: export.write",
    }
}

impl From<PoolError> for GwError {
    fn from(e: PoolError) -> Self {
        match e {
            // 마감에 닿았거나 읽기 연결이 고갈됨 — 재시도 가능
            PoolError::DeadlineExceeded | PoolError::ReadTimeout => Self::Busy,
            other => Self::internal(other),
        }
    }
}
