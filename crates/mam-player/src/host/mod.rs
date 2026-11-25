//! 플레이어 호스트 (Windows) — 워커 프로세스 실행·감시·자동 재시작·복원·타임아웃·이벤트 중계 (계약 §5.1).
//! 앱은 `PlayerHost` 와 `FrameSurface` 만 알면 된다. 상태 변경은 감독자 스레드 한 곳(`core`/`lifecycle`/`ops`)에서만 일어난다.

mod core;
pub mod events;
mod lifecycle;
mod link;
pub mod locate;
mod ops;
pub mod pipe;
pub mod proc;
pub mod restore;
mod supervisor;
pub mod surface;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
mod types;

pub use events::{
    Emit, EventBridge, EventSink, EventState, PlayerErrorCode, PlayerEvent, RestartInfo,
};
pub use proc::{ProcessSpawner, RawProcessHandle, WorkerConn, WorkerProcess, WorkerSpawner};
pub use supervisor::PlayerHost;
pub use surface::{BankAttach, FrameSurface, SurfaceError};
pub use types::{HostConfig, MAX_RESTARTS, OpenInfo, OpenRequest, Timeouts};

#[cfg(test)]
mod tests;
