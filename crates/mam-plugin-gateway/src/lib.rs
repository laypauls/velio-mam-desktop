//! 플러그인 호스트 API 게이트웨이 — `mam_plugin_host::HostApi` 구현. 권한 검사·자산 범위·KV·태그·로그 정제를 한 곳에 둔다.
//!
//! 이 crate 에는 SQL 이 없다: KV 는 `mam_db::plugin_kv`, 자산 읽기는 `mam_core::assets::detail`, 태그 쓰기는 `UserData::add_tags_low`.
//! 연결(P6): src-tauri 가 `Arc<Gateway>` 1개를 만들어 모든 `PluginHost` 에 `Arc<dyn HostApi>` 로 전달하고, 설치·권한 변경 뒤에
//! `grants().register`/`refresh` 를 부른다. 락 순서: 감독자 락 -> 레지스트리·계량기 락(말단, DB·호스트 호출 중 보유 금지).

mod asset;
mod errors;
mod gateway;
mod grants;
mod kv;
mod logs;
mod params;
mod quota;
mod record;
mod tag;

pub use errors::ALLOWED_MESSAGES;
pub use gateway::{Gateway, GatewayConfig};
pub use grants::{Grant, GrantRegistry, compute_grant};
pub use logs::{Admit, LogLimiter, LogSink, MAX_LOG_BYTES, TracingSink, sanitize};
pub use record::{Completion, MAX_OUTPUT_KV, RecordError, Recorded};
