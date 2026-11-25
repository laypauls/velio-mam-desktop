//! 플러그인 런타임 — 레지스트리·설치/제거·provider 호출(P6)과 작업 대기열 연동(P5)을 한 crate 에 둔다.
//!
//! `mam-core` 는 플러그인을 모른다(`JobKind::Plugin` 변종만). 이 crate 는 실제 SQLite·워커로 Tauri 없이 시험할 수 있고,
//! `src-tauri` 만 이 crate 에 의존한다. 모듈 소유: `api` 골격(동결) / `runtime`·`install` P6 / `jobs` P5.

pub mod api;
pub mod install;
pub mod jobs;
pub mod runtime;

pub use api::*;
pub use jobs::{PluginJobs, PluginJobsDeps};
pub use mam_plugin_gateway::GatewayConfig;
pub use mam_plugin_host::SystemClock as HostSystemClock;
pub use mam_plugin_manifest::clean_text;
pub use runtime::{PluginRuntime, RuntimeDeps, locate_worker, locate_worker_default};
