//! `WatchManager`·`DetectorRegistry` 시험 (가짜 감지기).
//! 분류: Happy / Business rule (폴백·사유 우선순위) / External (시작 실패) / Concurrency (해제 대기)

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use super::detectors::ScanOnly;
use super::manager::WatchManager;
use super::registry::{DetectorRegistry, FallbackReason};
use super::release::ReleaseToken;
use super::sink::ChangeSink;
use crate::volume::VolumeKind;

struct Fake {
    probe: ProbeResult,
    fail: AtomicBool,
    starts: AtomicUsize,
    /// 시작 때 토큰을 쥐어 여기에 보관한다 (비우면 OS 핸들이 닫힌 것)
    parked: Arc<Mutex<Vec<ReleaseToken>>>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            probe: ProbeResult::Available,
            fail: AtomicBool::new(false),
            starts: AtomicUsize::new(0),
            parked: Arc::default(),
        })
    }

    fn unavailable(key: &'static str) -> Arc<Self> {
        Arc::new(Self {
            probe: ProbeResult::Unavailable { reason_key: key },
            fail: AtomicBool::new(false),
            starts: AtomicUsize::new(0),
            parked: Arc::default(),
        })
    }
}

struct Handle;
impl WatchHandle for Handle {}

/// `Arc<Fake>` 를 감지기로 쓰기 위한 얇은 래퍼
struct FakeDetector(Arc<Fake>);

impl ChangeDetector for FakeDetector {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        self.0.probe
    }
    fn start(
        &self,
        _t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        self.0.starts.fetch_add(1, Ordering::SeqCst);
        self.0.parked.lock().unwrap().push(sink.release_token());
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(DetectError::Start("boom".into()));
        }
        Ok(Box::new(Handle))
    }
}

fn target(id: i64) -> WatchTarget {
    WatchTarget {
        folder_id: id,
        root: PathBuf::from("."),
        volume_kind: VolumeKind::Fixed,
    }
}

fn manager(fake: &Arc<Fake>, auto: &[&'static str]) -> WatchManager {
    let (tx, _rx) = channel();
    let all: Vec<Arc<dyn ChangeDetector>> = vec![Arc::new(FakeDetector(Arc::clone(fake))), Arc::new(ScanOnly)];
    WatchManager::new(tx, Arc::new(DetectorRegistry::new(all, auto)))
}

fn release_all(f: &Fake) {
    f.parked.lock().unwrap().clear();
}

#[test]
fn t_wmgr_01_settled_entry_makes_start_a_noop() {
    // Arrange
    let fake = Fake::new();
    let m = manager(&fake, &["fake"]);
    let first = m.start(&target(1), None, None);

    // Act
    let second = m.start(&target(1), None, None);

    // Assert
    assert!(first.started_now && !second.started_now);
    assert_eq!(fake.starts.load(Ordering::SeqCst), 1);
    assert_eq!(second.status.active, "fake");
    assert!(m.is_settled(1) && m.is_watching(1));
}

#[test]
fn t_wmgr_02_start_failure_falls_back_to_scan_only_and_is_retried() {
    // Arrange
    let fake = Fake::new();
    fake.fail.store(true, Ordering::SeqCst);
    let m = manager(&fake, &["fake"]);

    // Act
    let out = m.start(&target(1), None, None);

    // Assert
    assert_eq!(out.status.active, "scan_only");
    assert_eq!(out.status.fallback, Some(FallbackReason::StartFailed));
    assert!(out.last_error.is_some());
    assert!(m.is_watching(1) && !m.is_settled(1));

    // Act: 같은 폴더를 다시 시작하면 가짜를 다시 부른다
    let retry = m.start(&target(1), None, None);
    assert!(retry.started_now);
    assert_eq!(fake.starts.load(Ordering::SeqCst), 2);

    // Act: 이제 성공
    fake.fail.store(false, Ordering::SeqCst);
    let ok = m.start(&target(1), None, None);
    assert_eq!(ok.status.active, "fake");
    assert_eq!(ok.status.fallback, None);
    assert!(m.is_settled(1));
}

#[test]
fn t_wmgr_03_unknown_mode_uses_auto_order_with_reason() {
    let fake = Fake::new();
    let m = manager(&fake, &["fake"]);

    let out = m.start(&target(1), Some("removed_detector"), None);

    assert_eq!(out.status.active, "fake");
    assert_eq!(out.status.fallback, Some(FallbackReason::UnknownMode));
    assert_eq!(out.status.fallback.unwrap().key(), "unknown_mode");
}

#[test]
fn t_wmgr_04_unavailable_mode_falls_back_with_probe_reason() {
    let fake = Fake::unavailable("no_network");
    let m = manager(&fake, &["fake"]);

    let out = m.start(&target(1), Some("fake"), None);

    assert_eq!(out.status.active, "scan_only");
    assert_eq!(out.status.fallback, Some(FallbackReason::Unavailable("no_network")));
    assert_eq!(fake.starts.load(Ordering::SeqCst), 0);
}

#[test]
fn t_wmgr_05_invalid_options_fall_back_to_auto() {
    for raw in [r#"{"bad":1}"#, "not json", "[]"] {
        let fake = Fake::new();
        let m = manager(&fake, &["fake"]);

        let out = m.start(&target(1), Some("fake"), Some(raw));

        assert_eq!(out.status.fallback, Some(FallbackReason::InvalidOptions), "{raw}");
        assert_eq!(out.status.active, "fake", "auto order still starts the detector with defaults");
    }
}

#[test]
fn t_wmgr_06_scan_only_entry_does_not_hide_an_unreleased_handle() {
    // Arrange: A = scan_only(토큰 없음), B = fake(토큰을 쥔 채 멈춤) — 옛 `alive <= entries` 방식은 여기서 거짓 해제를 냈다
    let fake = Fake::new();
    let m = manager(&fake, &["fake"]);
    m.start(&target(2), Some("scan_only"), None);
    m.start(&target(1), None, None);

    // Act
    m.stop(1);

    // Assert
    assert!(!m.wait_released(Duration::from_millis(50)), "token of the stopped sink is still alive");
    release_all(&fake);
    assert!(m.wait_released(Duration::from_millis(50)));
    assert!(m.is_watching(2));
}

#[test]
fn t_wmgr_07_token_leaked_by_a_failed_start_is_still_waited_for() {
    // Arrange
    let fake = Fake::new();
    fake.fail.store(true, Ordering::SeqCst);
    let m = manager(&fake, &["fake"]);

    // Act
    m.start(&target(1), None, None);

    // Assert
    assert!(!m.wait_released(Duration::from_millis(50)));
    release_all(&fake);
    assert!(m.wait_released(Duration::from_millis(50)));
}

#[test]
fn t_wmgr_08_registry_rules() {
    // scan_only 는 항상 마지막 자동 후보다
    let fake = Fake::new();
    let reg = DetectorRegistry::new(vec![Arc::new(FakeDetector(fake))], &["scan_only", "fake"]);
    let plan = reg.plan(&target(1), None, None);
    let ids: Vec<_> = plan.candidates.iter().map(|c| c.detector.id()).collect();
    assert_eq!(ids, vec!["fake", "scan_only"]);
    assert!(reg.get("scan_only").is_some() && reg.get("nope").is_none());
    assert_eq!(reg.all().count(), 2);
}

#[test]
#[should_panic(expected = "duplicate detector id")]
fn t_wmgr_08_duplicate_ids_panic() {
    let fake = Fake::new();
    let _ = DetectorRegistry::new(
        vec![Arc::new(FakeDetector(Arc::clone(&fake))), Arc::new(FakeDetector(fake))],
        &["fake"],
    );
}

#[test]
#[should_panic(expected = "unregistered")]
fn t_wmgr_08_unknown_auto_id_panics() {
    let _ = DetectorRegistry::new(vec![Arc::new(ScanOnly)], &["ghost"]);
}

#[test]
fn t_wmgr_09_first_reason_wins_over_later_start_failure() {
    // Arrange: 알 수 없는 방식(사유 1) + 자동 후보 시작 실패(사유 2) — 사유는 첫 번째로 남는다
    let fake = Fake::new();
    fake.fail.store(true, Ordering::SeqCst);
    let m = manager(&fake, &["fake"]);

    // Act
    let out = m.start(&target(1), Some("ghost"), None);

    // Assert
    assert_eq!(out.status.active, "scan_only");
    assert_eq!(out.status.fallback, Some(FallbackReason::UnknownMode));
    assert!(out.last_error.is_some() && !m.is_settled(1), "start failure is tracked apart from the displayed reason");
}

#[test]
fn t_wmgr_10_start_failure_behind_another_reason_is_still_retried() {
    // Arrange: 알 수 없는 방식(표시 사유) + 자동 후보 시작 실패 — 표시 사유와 무관하게 재시도 대상이다
    let fake = Fake::new();
    fake.fail.store(true, Ordering::SeqCst);
    let m = manager(&fake, &["fake"]);
    let first = m.start(&target(1), Some("ghost"), None);
    assert_eq!(first.status.fallback, Some(FallbackReason::UnknownMode));
    assert!(m.is_watching(1) && !m.is_settled(1));

    // Act: 다음 점검의 재시도
    let retry = m.start(&target(1), Some("ghost"), None);

    // Assert
    assert!(retry.started_now);
    assert_eq!(fake.starts.load(Ordering::SeqCst), 2);

    // 복구되면 정착하고 표시 사유는 그대로
    fake.fail.store(false, Ordering::SeqCst);
    let ok = m.start(&target(1), Some("ghost"), None);
    assert_eq!((ok.status.active, ok.status.fallback), ("fake", Some(FallbackReason::UnknownMode)));
    assert!(m.is_settled(1));
}

/// 버려지는 동안(= OS 핸들을 닫는 중) 멈춰 서는 핸들 — 제거와 해제 사이의 틈을 결정적으로 만든다
struct GateHandle {
    dropping: std::sync::mpsc::Sender<()>,
    gate: std::sync::mpsc::Receiver<()>,
}

impl WatchHandle for GateHandle {}

impl Drop for GateHandle {
    fn drop(&mut self) {
        let _ = self.dropping.send(());
        let _ = self.gate.recv_timeout(Duration::from_secs(10));
    }
}

struct GateDetector {
    parked: Arc<Mutex<Vec<ReleaseToken>>>,
    handle: Mutex<Option<GateHandle>>,
}

impl ChangeDetector for GateDetector {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(&self, _t: &WatchTarget, _o: &DetectorOptions, sink: ChangeSink) -> Result<Box<dyn WatchHandle>, DetectError> {
        self.parked.lock().unwrap().push(sink.release_token());
        let h = self.handle.lock().unwrap().take().expect("one start");
        Ok(Box::new(h))
    }
}

#[test]
fn t_wmgr_11_sink_is_closed_atomically_with_entry_removal() {
    // Arrange: 엔트리 제거 직후 핸들을 닫는 중인(drop 이 멈춘) 상태에서 해제 대기를 건다
    let (dropping_tx, dropping_rx) = channel();
    let (gate_tx, gate_rx) = channel();
    let parked: Arc<Mutex<Vec<ReleaseToken>>> = Arc::default();
    let det = GateDetector {
        parked: Arc::clone(&parked),
        handle: Mutex::new(Some(GateHandle { dropping: dropping_tx, gate: gate_rx })),
    };
    let (tx, _rx) = channel();
    let m = Arc::new(WatchManager::new(tx, Arc::new(DetectorRegistry::new(vec![Arc::new(det)], &["fake"]))));
    m.start(&target(1), None, None);

    // Act: 다른 스레드가 stop — 핸들 drop 에서 멈춘다
    let m2 = Arc::clone(&m);
    let stopper = std::thread::spawn(move || m2.stop(1));
    dropping_rx.recv_timeout(Duration::from_secs(5)).expect("handle drop started");

    // Assert: 엔트리는 이미 없고(stop 은 no-op 으로 보임) 토큰이 살아 있으니 해제됐다고 말하면 안 된다
    assert!(!m.is_watching(1));
    assert!(!m.wait_released(Duration::from_millis(50)), "closed sink with a live token must block");
    parked.lock().unwrap().clear();
    gate_tx.send(()).unwrap();
    stopper.join().unwrap();
    assert!(m.wait_released(Duration::from_millis(500)));
}
