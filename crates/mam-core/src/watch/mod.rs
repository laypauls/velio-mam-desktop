//! 실시간 감시 (P2-2, D5) — 감지기 어댑터가 보고한 이벤트를 경로 단위로 디바운스한 뒤 단건 반영한다.
//! 감시 오류·버퍼 넘침은 폴더 재스캔으로 만회한다. NAS 는 이벤트가 오지 않을 수 있어 주기 재스캔이 주 수단이다.
//!
//! 구조: `ChangeDetector`(어댑터, 컴파일 타임) -> `DetectorRegistry`(선택·폴백) -> `WatchManager`(폴더별 시작·중지)
//! -> `ChangeSink`(메시지) -> 변경 대기열(`ingest::pending`, SQLite). 해제 대기는 `ReleaseTracker`(ADR-1).
//!
//! 다른 OS (설계 메모 — 제품 범위는 Windows 10/11 뿐이라 구현하지 않는다, `device_watch.rs` 와 같은 방식):
//! - macOS: FSEvents 어댑터(`notify` 의 `RecommendedWatcher` 가 FSEvents 를 쓴다) 를 `detectors/` 에 추가하고 `platform_default()` 에 등록.
//! - Linux: inotify 어댑터(같은 `notify` 래퍼). 감시 수 상한(`fs.inotify.max_user_watches`) 초과는 시작 실패 -> `scan_only` 폴백.

mod debounce;
pub mod detector;
pub mod detectors;
mod health;
mod manager;
mod options_ref;
mod registry;
mod release;
pub mod remote;
mod sink;

#[cfg(test)]
mod contract;
#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod manager_tests;

pub use debounce::Debouncer;
pub use detector::{
    COMMON_OPTION_FIELDS, ChangeDetector, ChangeDetectorInfo, DetectError, DetectorOptions,
    EXCLUSIVE_OPEN_CHECK, OptionField, OptionKind, ProbeResult, WatchHandle, WatchTarget,
    exclusive_open_check_of, parse_integer_options, parse_options_checked,
};
pub use detectors::{DdpAudit, FanotifyFeed, OsEvents, Polling, ScanOnly, convert};
pub use health::SourceHealth;
pub use manager::{StartOutcome, WatchManager, WatchStatus};
pub use options_ref::{NoRefs, RefResolver};
pub use registry::{Candidate, DetectorRegistry, FallbackReason, Plan};
pub use release::{ReleaseToken, SinkId};
pub use sink::{ChangeSink, WatchMsg};
