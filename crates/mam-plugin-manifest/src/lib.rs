//! 플러그인 manifest(`plugin.json`)·권한·`when` 식 — 순수 crate(파일 IO·실행 없음).
//!
//! 모든 진입점은 `&str` 또는 파싱된 값을 받는다. 파일 읽기, 경로 canonicalize,
//! 심볼릭 링크·정션 탈출 확인, 파일 존재 확인은 설치기(P6)의 몫이다.
//!
//! `engines.mam` 은 1.0 전에는 caret(`^0.1` 은 앱 0.2.0 에서 깨진다) 대신 `>=` 를 권장한다.

mod contrib_types;
mod contributions;
mod engine;
mod error;
mod ids;
mod l10n;
mod main_path;
mod manifest;
mod permission;
mod raw;
mod text;
pub mod when;

pub use contrib_types::{
    ActivationEvent, AssetKind, Command, Contributes, ExportProvider, MenuItem, Menus, Provider,
    Providers,
};
pub use contributions::{EXPORT_EXTENSIONS, export_extension_allowed};
pub use engine::check_engine;
pub use error::{EngineError, LocaleError, MainPathError, ManifestError};
pub use ids::{CommandId, PluginId, ProviderId, TitleKey, is_reserved_name};
pub use l10n::{LocaleBundle, LocaleMap, parse_locale_file};
pub use main_path::MainPath;
pub use manifest::{Engines, Manifest, parse_manifest};
pub use permission::Permission;
pub use text::{clean_text, is_format_char};
pub use when::{WhenContext, WhenError, WhenErrorKind, WhenEvalError, WhenExpr, WhenValue};

pub const MANIFEST_FILE_NAME: &str = "plugin.json";
pub const L10N_DIR: &str = "l10n";
pub const SUPPORTED_API_VERSIONS: &[u32] = &[1];

/// 제안 한도(미측정). 모두 바이트 또는 개수.
pub mod limits {
    pub const MAX_MANIFEST_BYTES: usize = 65_536;
    pub const MAX_ID_LEN: usize = 128;
    pub const MAX_COMMAND_ID_LEN: usize = 64;
    pub const MAX_TITLE_KEY_LEN: usize = 128;
    pub const MAX_VERSION_LEN: usize = 64;
    pub const MAX_MAIN_LEN: usize = 260;
    pub const MAX_MAIN_COMPONENTS: usize = 16;
    pub const MAX_COMMANDS: usize = 32;
    pub const MAX_MENU_ITEMS: usize = 32;
    pub const MAX_PROVIDERS_PER_KIND: usize = 16;
    pub const MAX_PROVIDERS_TOTAL: usize = 32;
    pub const MAX_ACTIVATION_EVENTS: usize = 64;
    pub const MAX_WHEN_LEN: usize = 256;
    pub const MAX_WHEN_DEPTH: usize = 16;
    pub const MAX_LOCALE_BYTES: usize = 262_144;
    pub const MAX_LOCALE_ENTRIES: usize = 2000;
    pub const MAX_LOCALE_VALUE_LEN: usize = 512;
}
