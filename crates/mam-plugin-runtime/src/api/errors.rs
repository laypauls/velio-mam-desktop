//! 런타임 오류 — command 경계(`src-tauri/src/plugin_errors.rs`)가 계약 코드로 바꾼다. 메시지에 호스트 경로를 싣지 않는다.

use mam_db::PoolError;
use mam_plugin_host::{InvokeError, ScopeTooLarge};
use mam_plugin_manifest::{EngineError, LocaleError, ManifestError, Permission};

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("plugin not installed")]
    NotInstalled,
    /// 사용자가 껐거나 반복 결함으로 자동 비활성화됐다
    #[error("plugin disabled")]
    Disabled,
    #[error("unknown command or provider")]
    UnknownTarget,
    #[error("permission not granted")]
    PermissionDenied(Permission),
    #[error(transparent)]
    Scope(#[from] ScopeTooLarge),
    #[error(transparent)]
    Invoke(#[from] InvokeError),
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("path must be absolute")]
    NotAbsolute,
    #[error("path is not a directory")]
    NotADirectory,
    #[error("path overlaps the app data folder")]
    OverlapsDataDir,
    #[error("plugin.json not found")]
    ManifestMissing,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("invalid locale file `{file_stem}`: {source}")]
    Locale {
        file_stem: String,
        source: LocaleError,
    },
    #[error("main script not found")]
    MainMissing,
    #[error("main script is outside the plugin folder")]
    MainEscapes,
    #[error("main script is too large")]
    MainTooLarge,
    #[error("main script is not valid UTF-8")]
    MainNotUtf8,
    #[error("too many locale files")]
    TooManyLocaleFiles,
    #[error("plugin changed since it was inspected")]
    PreviewMismatch,
    #[error("unknown or undeclared permission")]
    UnknownPermission,
    #[error("plugin not installed")]
    NotInstalled,
    #[error("another plugin operation is in progress")]
    Busy,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] PoolError),
}
