//! 호스트 ↔ 플레이어 워커 계약 — OS 의존성이 없는 순수 타입·로직 (호스트와 워커 crate 가 함께 쓴다).
//! 정본: `docs/requests/2026-11/20261119-192800-architect-libmpv-ipc-contract.md` §1~§3, §5.
//!
//! - 제어 채널은 줄 단위 JSON 이다 (한 줄 = 메시지 1개). 명령은 `cmd` 태그, 워커 메시지는 `ev` 태그
//! - 프레임 전달(WebView ↔ 호스트)은 `frames` 의 `"mam": "player.v1"` 객체 메시지다

pub mod command;
pub mod edl;
pub mod frame_math;
pub mod frames;
pub mod message;
pub mod slots;
pub mod wire;

pub use command::{Command, ControlAction, RangeMs, Request};
pub use edl::{EdlMap, Landing, build_edl};
pub use frame_math::{
    LastFrameGuard, RENDER_CAP, back_cache_frames, fit_render_size, is_long_gop, ms_from_secs, snap_to_frame_start, stride_for, volume_to_mpv,
};
pub use frames::{
    BufferInfo, FrameMsgOut, MAX_RELEASE_BODY, MAX_RELEASE_SLOTS, PLAYER_WIRE_TAG, PixelFormat, ReleaseMsgIn, ReleaseReject, SLOT_COUNT,
    SlotRef, parse_release, validate_release_value,
};
pub use message::{Message, ProbeData, RenderPath, WorkerErrorCode};
pub use slots::{Outcome, SlotState, SlotTable};
pub use wire::{WireError, decode_line, encode_line, read_bounded_line};

/// 호스트·워커가 맞아야 하는 프로토콜 버전 — 워커 `Ready.protocol` 과 다르면 `player_unavailable`
pub const PROTOCOL_VERSION: u32 = 1;
