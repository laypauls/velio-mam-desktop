//! 플러그인 런타임 오류 -> command 경계 오류 (계약 `ErrorCode`). 매핑은 동결 — 명세 §C3 오류 표.
//!
//! 메시지는 고정 문구이거나 manifest crate 의 Display(플러그인 제공 값은 64자로 잘림)이고 호스트 경로를 싣지 않는다.
//! 플러그인이 만든 텍스트(`InvokeError::Script` 등)는 응답에 싣지 않고 로그에만 남긴다 — `plugin_error` 의 메시지는 정적 i18n 키다.

use mam_plugin_runtime::{InstallError, InvokeError, ManifestError, RunError};

use crate::error::{CommandError, ErrorCode};

const INCOMPATIBLE: &str = "plugin is not compatible with this app version";

/// 로그·메시지에 넣는 플러그인 제공 문자열은 64자에서 자른다
fn short(s: &str) -> String {
    let cleaned = mam_plugin_runtime::clean_text(s);
    let mut it = cleaned.chars();
    let head: String = it.by_ref().take(64).collect();
    if it.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn is_disk_full(e: &std::io::Error) -> bool {
    // ERROR_DISK_FULL(112), ERROR_HANDLE_DISK_FULL(39)
    e.kind() == std::io::ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(112 | 39))
}

impl From<InstallError> for CommandError {
    fn from(e: InstallError) -> Self {
        use ErrorCode::{
            Busy, DiskFull, InvalidInput, Io, NotFound, PluginIncompatible, PluginInvalid,
        };
        match e {
            InstallError::Manifest(ManifestError::UnsupportedApiVersion { .. })
            | InstallError::Engine(_) => Self::new(PluginIncompatible, INCOMPATIBLE),
            InstallError::Manifest(m) => Self::new(PluginInvalid, m.to_string()),
            InstallError::Locale { file_stem, source } => Self::new(
                PluginInvalid,
                format!("invalid locale file `{}`: {source}", short(&file_stem)),
            ),
            InstallError::ManifestMissing
            | InstallError::MainMissing
            | InstallError::MainEscapes
            | InstallError::MainTooLarge
            | InstallError::MainNotUtf8
            | InstallError::TooManyLocaleFiles => Self::new(PluginInvalid, e.to_string()),
            InstallError::NotAbsolute => Self::new(InvalidInput, "path must be absolute"),
            InstallError::NotADirectory => Self::new(InvalidInput, "path is not a directory"),
            InstallError::OverlapsDataDir => {
                Self::new(InvalidInput, "path overlaps the app data folder")
            }
            InstallError::PreviewMismatch => {
                Self::new(InvalidInput, "plugin changed since it was inspected")
            }
            InstallError::UnknownPermission => {
                Self::new(InvalidInput, "unknown or undeclared permission")
            }
            InstallError::NotInstalled => Self::new(NotFound, "plugin not installed"),
            InstallError::Busy => Self::new(Busy, "another plugin operation is in progress"),
            InstallError::Io(io) if is_disk_full(&io) => {
                Self::logged(DiskFull, "not enough disk space", &io)
            }
            InstallError::Io(io) => Self::logged(Io, "file operation failed", &io),
            InstallError::Db(p) => p.into(),
        }
    }
}

impl From<RunError> for CommandError {
    fn from(e: RunError) -> Self {
        use ErrorCode::{
            InvalidInput, NotFound, PluginDisabled, PluginError, PluginPermissionDenied,
            PluginTimeout,
        };
        match e {
            RunError::NotInstalled => Self::new(NotFound, "plugin not installed"),
            RunError::UnknownTarget => Self::new(NotFound, "unknown command or provider"),
            RunError::Disabled | RunError::Invoke(InvokeError::Disabled) => {
                Self::new(PluginDisabled, "plugin is disabled")
            }
            RunError::PermissionDenied(_) => {
                Self::new(PluginPermissionDenied, "permission not granted")
            }
            RunError::Scope(_) => Self::new(InvalidInput, "too many assets"),
            RunError::Invoke(InvokeError::Timeout) => {
                Self::new(PluginTimeout, "plugin call timed out")
            }
            RunError::Invoke(other) => {
                // 플러그인이 만든 텍스트는 로그에만 — 응답은 정적 i18n 키
                tracing::warn!(error = %short(&other.to_string()), "plugin call failed");
                Self::new(PluginError, other.key())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation(로그 줄 위조·방향 재정의) / Boundary(64자 절단)

    #[test]
    fn t_ipc_perr_01_short_neutralizes_control_and_format_chars() {
        let s = short("boom\nERROR forged line\r\u{202E}txt.exe\u{200B}");
        assert!(
            !s.contains('\n')
                && !s.contains('\r')
                && !s.contains('\u{202E}')
                && !s.contains('\u{200B}'),
            "{s:?}"
        );
        assert!(s.starts_with("boom ERROR forged line"));
        assert_eq!(
            short(&"a".repeat(70)).chars().count(),
            65,
            "64 chars + ellipsis"
        );
    }
}
