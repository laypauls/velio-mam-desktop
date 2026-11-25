//! 신호원 상태(`SourceHealth`)와 sink 보고 시험 (T-health-01 ~ 07, T-subtree-01).
//! 분류: Happy (01 신호 시각·05 os_events 보고) / Boundary (03 만료 경계) / Business rule (02 저하 유지·복구, 06 scan_only 무보고) /
//! Concurrency (04 낡은 sink 무시) / 계약 (07 JSON 모양, T-subtree-01 메시지 변종). 시계는 주입(수동 전진).

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use super::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use super::detectors::ScanOnly;
use super::detectors::os_events::report_health;
use super::health::SourceHealth;
use super::manager::WatchManager;
use super::registry::DetectorRegistry;
use super::sink::{ChangeSink, WatchMsg};
use crate::SharedClock;
use crate::volume::VolumeKind;

struct Clock(AtomicI64);

impl mam_db::Clock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 시작할 때 받은 sink 를 보관해 시험이 감지기 대신 보고하게 하는 감지기
struct Parking(Arc<Mutex<Option<ChangeSink>>>);

struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Parking {
    fn id(&self) -> &'static str {
        "parking"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        _t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        *self.0.lock().unwrap() = Some(sink);
        Ok(Box::new(Handle))
    }
}

struct Fixture {
    clock: Arc<Clock>,
    sink: Arc<Mutex<Option<ChangeSink>>>,
    m: WatchManager,
    rx: Receiver<WatchMsg>,
}

const T0: i64 = 1_700_000_000_000;

fn fixture(auto: &[&'static str]) -> Fixture {
    let clock = Arc::new(Clock(AtomicI64::new(T0)));
    let sink = Arc::new(Mutex::new(None));
    let (tx, rx) = channel();
    let all: Vec<Arc<dyn ChangeDetector>> =
        vec![Arc::new(Parking(Arc::clone(&sink))), Arc::new(ScanOnly)];
    let shared: SharedClock = clock.clone();
    let m = WatchManager::with_clock(tx, Arc::new(DetectorRegistry::new(all, auto)), shared);
    Fixture { clock, sink, m, rx }
}

fn target() -> WatchTarget {
    WatchTarget {
        folder_id: 1,
        root: PathBuf::from("."),
        volume_kind: VolumeKind::Fixed,
    }
}

impl Fixture {
    fn start(&self) -> ChangeSink {
        self.m.start(&target(), None, None);
        self.sink.lock().unwrap().clone().expect("detector started")
    }
    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }
}

#[test]
fn t_health_01_started_detector_is_ok_without_a_signal_and_a_signal_stamps_the_injected_time() {
    // Arrange
    let f = fixture(&["parking"]);
    let sink = f.start();
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: None
        })
    );

    // Act
    f.advance(5_000);
    sink.signal();

    // Assert
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: Some(T0 + 5_000)
        })
    );
    assert_eq!(f.m.health(2), None, "an unwatched folder has no health");
}

#[test]
fn t_health_02_degraded_keeps_its_start_time_until_recovered_and_a_signal_does_not_clear_it() {
    // Arrange
    let f = fixture(&["parking"]);
    let sink = f.start();

    // Act
    sink.degraded("watch.health.reconnecting");
    f.advance(10_000);
    sink.degraded("watch.health.slow");
    f.advance(10_000);
    sink.signal();

    // Assert: 시작 시각은 처음 것, 사유는 최신, 신호가 와도 복구 전까지 저하
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0,
            reason_key: "watch.health.slow"
        })
    );
    sink.recovered();
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: Some(T0 + 20_000)
        })
    );
}

#[test]
fn t_health_03_a_timed_degraded_state_expires_exactly_at_its_hold_time() {
    // Arrange
    let f = fixture(&["parking"]);
    let sink = f.start();
    sink.degraded_for("watch.health.overflow", 60_000);

    // Act / Assert: 59.999 초까지 저하, 정확히 60 초에 정상
    f.advance(59_999);
    assert!(matches!(f.m.health(1), Some(SourceHealth::Degraded { since, .. }) if since == T0));
    f.advance(1);
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: None
        })
    );
}

#[test]
fn t_health_04_reports_from_a_replaced_or_stopped_sink_are_ignored() {
    // Arrange: 첫 감시를 멈추고 다시 시작 — 낡은 sink 는 새 상태를 건드릴 수 없다
    let f = fixture(&["parking"]);
    let old = f.start();
    f.m.stop(1);
    assert_eq!(f.m.health(1), None);
    old.degraded("watch.health.late");
    assert_eq!(f.m.health(1), None, "a stopped sink cannot create state");
    let fresh = f.start();

    // Act
    old.degraded("watch.health.late");
    f.advance(1_000);
    fresh.signal();

    // Assert
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: Some(T0 + 1_000)
        })
    );
    f.m.stop_all();
    assert_eq!(f.m.health(1), None);
}

#[test]
fn t_health_05_os_events_reports_overflow_and_errors_as_degraded_and_changes_as_signals() {
    // Arrange
    let f = fixture(&["parking"]);
    let sink = f.start();
    let path = vec![WatchMsg::Path {
        folder_id: 1,
        path: PathBuf::from("a"),
    }];
    let rescan = vec![WatchMsg::Rescan { folder_id: 1 }];

    // Act / Assert: 변경 -> 신호
    f.advance(1_000);
    report_health(&sink, false, &path);
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: Some(T0 + 1_000)
        })
    );
    // 변경이 없는 이벤트(접근 등)는 아무것도 바꾸지 않는다
    f.advance(1_000);
    report_health(&sink, false, &[]);
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: Some(T0 + 1_000)
        })
    );
    // 넘침 -> 짧게 저하, 곧 정상
    report_health(&sink, false, &rescan);
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0 + 2_000,
            reason_key: "watch.health.overflow"
        })
    );
    f.advance(60_000);
    assert!(matches!(f.m.health(1), Some(SourceHealth::Ok { .. })));
    // 감시 오류는 사유 키가 다르다
    report_health(&sink, true, &rescan);
    assert!(matches!(
        f.m.health(1),
        Some(SourceHealth::Degraded {
            reason_key: "watch.health.error",
            ..
        })
    ));
}

#[test]
fn t_health_06_scan_only_reports_nothing_and_stays_ok() {
    // Arrange / Act
    let f = fixture(&["scan_only"]);
    f.m.start(&target(), None, None);

    // Assert
    assert_eq!(
        f.m.health(1),
        Some(SourceHealth::Ok {
            last_signal_at: None
        })
    );
}

#[test]
fn t_health_07_json_uses_a_state_tag_and_camel_case_fields() {
    let ok = serde_json::to_value(SourceHealth::Ok {
        last_signal_at: Some(5),
    })
    .unwrap();
    assert_eq!(ok, serde_json::json!({"state": "ok", "lastSignalAt": 5}));
    let none = serde_json::to_value(SourceHealth::Ok {
        last_signal_at: None,
    })
    .unwrap();
    assert_eq!(
        none,
        serde_json::json!({"state": "ok", "lastSignalAt": null})
    );
    let bad = serde_json::to_value(SourceHealth::Degraded {
        since: 7,
        reason_key: "watch.health.overflow",
    })
    .unwrap();
    assert_eq!(
        bad,
        serde_json::json!({"state": "degraded", "since": 7, "reasonKey": "watch.health.overflow"})
    );
}

#[test]
fn t_subtree_01_sink_subtree_sends_a_subtree_message_for_its_own_folder() {
    // Arrange
    let f = fixture(&["parking"]);
    let sink = f.start();

    // Act
    sink.subtree(PathBuf::from("lib/new-dir"));
    sink.path(PathBuf::from("lib/a.jpg"));

    // Assert: Path 와 구분되는 변종이고 폴더 ID 는 sink 의 것
    let msgs: Vec<_> = f.rx.try_iter().collect();
    assert_eq!(
        msgs,
        vec![
            WatchMsg::Subtree {
                folder_id: 1,
                dir: PathBuf::from("lib/new-dir")
            },
            WatchMsg::Path {
                folder_id: 1,
                path: PathBuf::from("lib/a.jpg")
            },
        ]
    );
    assert!(msgs.iter().all(|m| m.folder_id() == 1));
}
