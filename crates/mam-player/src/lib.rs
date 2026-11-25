//! 플레이어 호스트 — libmpv 워커 프로세스(`mam-player-worker`) + WebView2 SharedBuffer 프레임 전달.
//!
//! - `protocol`: 호스트 ↔ 워커 계약 (OS 의존성 없음 — 워커 crate 도 쓴다)
//! - `host` (Windows): 워커 실행·감시·자동 재시작·상태 복원·명령 타임아웃·이벤트 중계 (`PlayerHost`)
//! - 원본은 워커가 읽기 전용으로만 연다. 이 crate 는 원본에 아무것도 쓰지 않는다

mod error;
#[cfg(windows)]
pub mod host;
pub mod protocol;
mod ranges;

pub use error::PlayerError;
pub use ranges::{MAX_RANGES, PlayRange, normalize as normalize_ranges, valid_ranges};
