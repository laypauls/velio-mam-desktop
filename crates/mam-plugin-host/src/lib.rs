//! 플러그인 워커 감독자(호스트 쪽). 워커 바이너리는 `mam-plugin-worker` — 이 crate 는 그 프로세스를 띄우고 감시한다.
//! 프로토콜 타입(`protocol`)은 워커 crate 도 쓴다. 파일·DB·네트워크는 건드리지 않는다 (호스트 함수는 `HostApi` 뒤).

mod activity;
mod breaker;
mod call_loop;
mod error;
mod host_api;
mod job;
mod line_reader;
pub mod protocol;
mod supervisor;
mod worker_proc;

pub use activity::HostActivity;
pub use breaker::{Breaker, BreakerConfig, Clock, SystemClock};
pub use error::InvokeError;
pub use host_api::{CallCx, HostApi, HostError, InvokeScope, MAX_SCOPE_ASSETS, ScopeTooLarge, codes};
pub use protocol::{InvokeKind, Limits, LogLevel};
pub use supervisor::{HostConfig, HostState, PluginHost};
