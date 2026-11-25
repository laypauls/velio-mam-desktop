use thiserror::Error;

use crate::permission::Permission;
use crate::when::WhenError;

/// 오류 메시지에 들어가는 플러그인 제공 값은 64자에서 자른다(로그 폭주 방지).
pub(crate) fn trunc(s: &str) -> String {
    const MAX: usize = 64;
    let cleaned = crate::text::clean_text(s);
    let mut it = cleaned.chars();
    let head: String = it.by_ref().take(MAX).collect();
    if it.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub(crate) fn required<T>(v: Option<T>, path: &str) -> Result<T, ManifestError> {
    v.ok_or_else(|| ManifestError::MissingField {
        path: path.to_owned(),
    })
}

#[derive(Debug, Error, PartialEq)]
pub enum ManifestError {
    #[error("manifest is too large: {len} bytes (max {max})")]
    TooLarge { len: usize, max: usize },
    #[error("invalid JSON at line {line}, column {column}: {}", trunc(.message))]
    Json {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("unsupported apiVersion: {}", trunc(.found))]
    UnsupportedApiVersion { found: String },
    #[error("missing required field `{path}`")]
    MissingField { path: String },
    #[error("invalid id at `{path}`: {}", trunc(.value))]
    InvalidId { path: String, value: String },
    #[error("invalid title key at `{path}`: {}", trunc(.value))]
    InvalidTitleKey { path: String, value: String },
    #[error("invalid version: {}", trunc(.value))]
    InvalidVersion { value: String },
    #[error("invalid version requirement: {}", trunc(.value))]
    InvalidVersionReq { value: String },
    #[error("invalid main `{}`: {reason}", trunc(.value))]
    InvalidMain {
        value: String,
        reason: MainPathError,
    },
    #[error("unknown permission: {}", trunc(.value))]
    UnknownPermission { value: String },
    #[error("duplicate permission: {}", trunc(.value))]
    DuplicatePermission { value: String },
    #[error("duplicate id at `{path}`: {}", trunc(.id))]
    DuplicateId { path: String, id: String },
    #[error("unknown command reference at `{path}`: {}", trunc(.command))]
    UnknownCommandRef { path: String, command: String },
    #[error("unknown provider reference at `{path}`: {}", trunc(.provider))]
    UnknownProviderRef { path: String, provider: String },
    #[error("invalid activation event: {}", trunc(.value))]
    InvalidActivationEvent { value: String },
    #[error("invalid asset kind at `{path}`: {}", trunc(.value))]
    InvalidAssetKind { path: String, value: String },
    #[error("invalid file extension at `{path}`: {}", trunc(.value))]
    InvalidFileExtension { path: String, value: String },
    #[error("value at `{path}` is too long (max {max})")]
    TooLong { path: String, max: usize },
    #[error("too many entries at `{path}` (max {max})")]
    TooMany { path: String, max: usize },
    #[error("invalid when expression at `{path}`: {error}")]
    InvalidWhen { path: String, error: WhenError },
    #[error("`{path}` requires permission `{permission}`")]
    MissingPermissionFor {
        path: String,
        permission: Permission,
    },
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum MainPathError {
    #[error("empty path")]
    Empty,
    #[error("absolute path")]
    Absolute,
    #[error("backslash is not allowed")]
    Backslash,
    #[error("colon is not allowed")]
    Colon,
    #[error("empty path component")]
    EmptyComponent,
    #[error("`.` component is not allowed")]
    DotComponent,
    #[error("`..` component is not allowed")]
    ParentEscape,
    #[error("illegal character")]
    IllegalChar,
    #[error("component ends with a dot or space")]
    TrailingDotOrSpace,
    #[error("reserved Windows name")]
    ReservedName,
    #[error("path must end with `.js`")]
    NotJs,
    #[error("path is too long")]
    TooLong,
    #[error("too many path components")]
    TooManyComponents,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EngineError {
    #[error("plugin requires MAM `{}` but this is {}", trunc(.required), trunc(.app))]
    Incompatible { required: String, app: String },
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LocaleError {
    #[error("locale file is too large")]
    TooLarge,
    #[error("invalid JSON at line {line}, column {column}: {}", trunc(.message))]
    Json {
        line: usize,
        column: usize,
        message: String,
    },
    #[error("value for key `{}` is not a string", trunc(.key))]
    NotFlatString { key: String },
    #[error("invalid locale key: {}", trunc(.key))]
    InvalidKey { key: String },
    #[error("value for key `{}` is too long", trunc(.key))]
    ValueTooLong { key: String },
    #[error("too many locale entries")]
    TooManyEntries,
    #[error("invalid locale code: {}", trunc(.code))]
    InvalidLocaleCode { code: String },
}
