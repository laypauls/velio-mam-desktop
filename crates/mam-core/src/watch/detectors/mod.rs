//! 내장 감지기 어댑터. 새 감지기는 여기에 파일을 추가하고 `DetectorRegistry::platform_default()` 에 등록한다.

pub mod ddp_audit;
pub mod fanotify_feed;
pub mod os_events;
pub mod polling;
pub mod scan_only;

pub use ddp_audit::DdpAudit;
pub use fanotify_feed::FanotifyFeed;
pub use os_events::{OsEvents, convert};
pub use polling::Polling;
pub use scan_only::ScanOnly;
