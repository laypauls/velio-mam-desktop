//! 보호 코어 (P2-9) — 비밀번호 인증, 원본 삭제(휴지통)·이동, 관리 데이터 초기화 계획.
//!
//! - [`ProtectionGuard`]: Argon2id 비밀번호 설정·변경·검증, 인증 면제 시간, 연속 실패 제한 (메모리 상태)
//! - [`delete_originals`] / [`move_originals`]: 인증 면제 안에서만 원본을 건드리는 유일한 경로.
//!   자산 변경 + revision + outbox 는 한 트랜잭션, 자산별 실패는 `failed` 목록으로
//! - [`Trash`]: 휴지통 이동 trait ([`RecycleBin`] 이 Windows 구현, 영구 삭제 없음)
//! - [`plan_reset`] / [`write_reset_marker`] / [`apply_pending_reset`]: 관리 데이터 초기화 (원본·백업 제외)
//!
//! PHC 해시 저장(settings)·백업 생성(`mam_db`)·Tauri command 는 호출자 몫이다.

mod delete;
mod error;
mod fsmove;
mod guard;
mod locate;
mod password;
mod paths;
mod relocate;
mod reset;
mod store;
mod trash;

pub use delete::{OriginalsEnv, delete_originals};
pub use error::{AssetFailure, OriginalsResult, ProtectError, ProtectionStatus};
pub use guard::{LOCKOUT_MS, MAX_FAILURES, ProtectionGuard, SINGLE_USE_WINDOW_MS};
pub use password::{MAX_PASSWORD_CHARS, MIN_PASSWORD_CHARS, validate_new_password};
pub use relocate::{MoveEnv, MoveReport, move_originals};
pub use reset::{
    BackupOutcome, ResetMarker, ResetPlan, ResetReport, apply_pending_reset, clear_reset_marker,
    marker_path, plan_reset, read_reset_marker, write_reset_marker,
};
#[cfg(windows)]
pub use trash::RecycleBin;
pub use trash::Trash;
pub use zeroize::Zeroizing;
