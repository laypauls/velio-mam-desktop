//! MAM 로컬 DB 계층 — SQLite 연결, 마이그레이션 러너, 경로 비교 키.
//!
//! 규칙 정본: `.claude/rules/db.md`. 스키마: `migrations/`.

mod backup;
mod backup_set;
mod connection;
mod error;
pub mod gallery_display;
pub mod ignored_volumes;
pub mod job_pause;
pub mod link_cleanup;
pub mod layout;
pub mod locale_code;
pub mod plugin_data;
pub mod plugin_kv;
pub mod plugin_registry;
pub mod plugin_settings;
pub mod plugin_tasks;
pub mod player_volume;
mod migrate;
mod path_key;
mod pool;
pub mod scene_cuts;
pub mod settings;
pub mod theme;
pub mod view_options;

pub use backup::{BackupMeta, inspect_backup, quick_check, restore_backup, snapshot};
pub use backup_set::{
    BackupInfo, BackupKind, SAFETY_BACKUPS_KEPT, create_backup, create_backup_with, list_backups,
    prune_backups,
};
pub use connection::{BUSY_TIMEOUT, open};
pub use error::DbError;
pub use gallery_display::{CardCheck, CardDateBy, CardInfoFields, CardSize, GalleryCardDisplay};
pub use layout::{DetailTab, LayoutSetting};
pub use player_volume::PlayerVolumeSetting;
pub use migrate::{Clock, MigrateReport, SystemClock, latest_version, migrate, migrate_with};
pub use path_key::{PathKeyError, path_key};
pub use pool::{DEFAULT_READERS, DbPool, PoolError, Priority};
pub use settings::{Settings, SettingsError};
pub use theme::{ThemeMode, ThemeSetting, ThemeSystemDark};
pub use view_options::{
    CapturedAtFallback, DoubleClickAction, GalleryPaging, IndexRailMode, PlayRangeKeys,
    SceneSensitivity, SortFieldsEnabled, ViewerPlacement,
};

/// 테스트·고급 사용을 위한 refinery 마이그레이션 타입 재노출.
pub use refinery::Migration;
