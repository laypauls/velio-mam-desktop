//! 수집 — 관리 폴더 등록, 디렉터리 순회 스캔(이어하기), 오류 기록, 폴더 트리 (P2-2).
//!
//! 순회는 디렉터리 단위 깊이 우선(pre-order)이고 형제는 이름 키 순이다. 이 순서가 결정적이므로 "마지막으로 완료한
//! 디렉터리"(cursor) 하나로 재개 지점을 표현할 수 있다. 원본은 읽기 전용으로만 연다.

mod error_types;
mod errors;
mod folder_relocate;
mod folder_settings;
mod folders;
mod guarded;
pub mod io_guard;
mod media_types;
mod progress;
mod run;
mod single;
mod tree;
mod walk;
mod work;

pub use error_types::IngestError;
pub use errors::{ScanErrorView, list_errors, record_error, resolve_stale, retry_targets};
pub use folder_relocate::{RelocateTarget, relocate_folder_tx};
pub use folder_settings::{DetectionView, set_rescan_policy, set_watch_mode};
pub use folders::{
    Availability, FolderRow, ManagedFolderView, ScanState, add_folder_row, count_under,
    detach_folder, hidden_folder_ids, list_folder_views, load_all, load_folder, root_keys,
    set_alias, set_enabled, set_hidden,
};
pub(crate) use folders::{TOTAL_COUNT_SQL, loc_in_hidden_folder, not_in_hidden_folder};
pub use guarded::{EntryKind, Fail, UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE};
pub use io_guard::{Guarded, IoGuard, IoGuardConfig};
pub use media_types::{
    COUNT_BY_TYPE_SQL, MediaTypeCounts, MediaTypeSet, count_by_type, set_media_types,
};
pub use progress::{STALL_AFTER_MS, ScanProgress, ScanProgressCell, ScanStall};
pub use run::{ScanExecutor, ScanOutcome, scan_folder};
pub use single::{PathAction, apply_path, apply_path_settled};
pub use tree::{FolderNode, list_subfolders};
pub use walk::{WalkRules, list_dir, preorder_cmp, subtree_before};
pub use work::{ScanCounts, ScanEnv};
