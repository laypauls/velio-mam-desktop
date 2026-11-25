//! 감지기 공통 계약 시험 — 새 감지기는 `detector_contract_tests!` 한 줄을 붙인다.
//! 분류: Happy / Validation (옵션) / Boundary (probe) / External (해제 대기) / 원본 보호.
//! 불변 조건 1·4 는 어댑터마다 이벤트 소스가 달라 어댑터 단위 시험이 맡는다 (os_events: `t_watch_conv_01/02`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use serde_json::json;

use super::detector::{ChangeDetector, ProbeResult, WatchTarget};
use super::detectors::{OsEvents, Polling, ScanOnly};
use super::manager::WatchManager;
use super::registry::DetectorRegistry;
use super::sink::WatchMsg;
use crate::scan::io_guard::IoGuardConfig;
use crate::volume::VolumeKind;

/// 출하된 감지기 ID — 새 감지기를 추가하면 여기에도 적는다 (ID 는 배포 후 rename 금지).
pub(super) const SHIPPED_DETECTOR_IDS: &[&str] = &["os_events", "polling", "scan_only"];

const COLLIDING_KEYS: &[&str] = &["unknown_mode", "invalid_options", "start_failed"];

pub(super) fn target(folder_id: i64, root: PathBuf, kind: VolumeKind) -> WatchTarget {
    WatchTarget {
        folder_id,
        root,
        volume_kind: kind,
    }
}

pub(super) fn manager_for(d: Arc<dyn ChangeDetector>) -> (WatchManager, Receiver<WatchMsg>) {
    let (tx, rx) = channel();
    let id = d.id();
    let registry = Arc::new(DetectorRegistry::new(vec![d], &[id]));
    (WatchManager::new(tx, registry), rx)
}

pub(super) fn check_id(d: &dyn ChangeDetector) {
    let id = d.id();
    let mut chars = id.chars();
    assert!(chars.next().is_some_and(|c| c.is_ascii_lowercase()), "{id}");
    assert!(chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{id}");
    assert!(SHIPPED_DETECTOR_IDS.contains(&id), "{id} must be listed in SHIPPED_DETECTOR_IDS");
}

pub(super) fn check_options(d: &dyn ChangeDetector) {
    assert!(d.parse_options(None).is_ok());
    for bad in [json!({"__x": 1}), json!([]), json!("a")] {
        assert!(d.parse_options(Some(&bad)).is_err(), "{bad}");
    }
}

pub(super) fn check_probe(d: &dyn ChangeDetector) {
    for kind in [VolumeKind::Fixed, VolumeKind::Removable, VolumeKind::Network] {
        let t = target(1, PathBuf::from("."), kind);
        let started = Instant::now();
        let r = d.probe(&t);
        assert!(started.elapsed() < Duration::from_millis(500), "probe must be cheap");
        if let ProbeResult::Unavailable { reason_key } = r {
            assert!(!COLLIDING_KEYS.contains(&reason_key), "{reason_key} collides");
        }
    }
}

fn mtime(p: &std::path::Path) -> SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).expect("mtime")
}

/// 불변 조건 2·3: 시작·중지가 원본을 바꾸지 않고, 중지 뒤 핸들이 유한 시간 안에 풀린다.
pub(super) fn check_release_and_original(d: Arc<dyn ChangeDetector>, emits: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let fixture = dir.path().join("orig.bin");
    std::fs::write(&fixture, b"original-bytes").expect("write");
    let before = (std::fs::read(&fixture).expect("read"), mtime(&fixture));
    let (m, _rx) = manager_for(d);
    let t = target(1, dir.path().to_path_buf(), VolumeKind::Fixed);

    let out = m.start(&t, None, None);
    assert!(out.started_now);
    if emits {
        assert!(m.live_tokens() >= 1, "an emitting detector holds a release token");
    }
    std::fs::write(dir.path().join("new.txt"), b"x").expect("write");
    m.stop(1);

    assert!(m.wait_released(Duration::from_secs(5)), "handle must be released after stop");
    let after = (std::fs::read(&fixture).expect("read"), mtime(&fixture));
    assert_eq!(before, after, "original must be untouched");
}

/// 변경을 내는 감지기: 파일 생성이 5초 안에 `WatchMsg::Path` 로 온다.
pub(super) fn check_emits_path(d: Arc<dyn ChangeDetector>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let (m, rx) = manager_for(d);
    m.start(&target(1, dir.path().to_path_buf(), VolumeKind::Fixed), None, None);
    std::fs::write(dir.path().join("created.txt"), b"x").expect("write");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = false;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(WatchMsg::Path { folder_id: 1, .. }) => {
                got = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    m.stop_all();
    assert!(got, "a created file must be reported");
}

/// 메시지 의미 (`Path`/`Subtree`/`Rescan`) — 감지기가 무엇을 내든 지켜야 한다.
/// - 폴더 ID 는 sink 의 폴더 (`emit` 의 debug_assert 와 별개로 수신 쪽에서도 확인)
/// - `Path`·`Subtree` 의 경로는 감시 루트 **아래**다 (루트 밖·루트 자신의 `Subtree` 금지 — 루트 전체는 `Rescan` 의 몫)
/// - 변경을 내지 않는 감지기(`emits = false`)는 아무 메시지도 내지 않는다
pub(super) fn check_message_semantics(d: Arc<dyn ChangeDetector>, emits: bool) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let (m, rx) = manager_for(d);
    m.start(&target(1, root.clone(), VolumeKind::Fixed), None, None);
    let sub = root.join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(sub.join("inner.txt"), b"x").expect("write");
    // 내는 감지기는 루트 아래 경로가 올 때까지, 안 내는 감지기는 짧게 기다린다
    let deadline = Instant::now() + Duration::from_secs(if emits { 5 } else { 1 });
    let mut msgs = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left.min(Duration::from_millis(200))) {
            Ok(msg) => {
                // 안쪽 파일을 직접 알리거나(`Path`), 그 폴더를 통째로 알리면(`Subtree`) 충분하다
                let inner = match &msg {
                    WatchMsg::Path { path, .. } => path.ends_with("inner.txt"),
                    WatchMsg::Subtree { dir, .. } => dir.ends_with("sub"),
                    WatchMsg::Rescan { .. } => false,
                };
                msgs.push(msg);
                if inner {
                    break;
                }
            }
            Err(_) if emits && msgs.is_empty() => {}
            Err(_) => break,
        }
    }
    m.stop_all();
    if !emits {
        assert!(msgs.is_empty(), "a detector that does not emit must stay silent: {msgs:?}");
        return;
    }
    assert!(!msgs.is_empty(), "an emitting detector must report the change");
    for msg in &msgs {
        assert_eq!(msg.folder_id(), 1, "{msg:?}");
        match msg {
            WatchMsg::Path { path, .. } => assert!(path.starts_with(&root), "{msg:?}"),
            WatchMsg::Subtree { dir, .. } => {
                assert!(dir.starts_with(&root) && dir != &root, "subtree must be below the root: {msg:?}")
            }
            WatchMsg::Rescan { .. } => {}
        }
    }
}

macro_rules! detector_contract_tests {
    ($module:ident, $make:expr, emits_events = $emits:expr) => {
        mod $module {
            use std::sync::Arc;

            use super::*;
            use crate::watch::contract as c;

            #[test]
            fn t_chgdet_01_id_is_stable_and_shipped() {
                c::check_id(&$make);
            }

            #[test]
            fn t_chgdet_02_options_defaults_ok_and_garbage_rejected() {
                c::check_options(&$make);
            }

            #[test]
            fn t_chgdet_03_probe_is_cheap_and_reason_keys_do_not_collide() {
                c::check_probe(&$make);
            }

            #[test]
            fn t_chgdet_04_05_stop_releases_handle_and_original_is_untouched() {
                c::check_release_and_original(Arc::new($make), $emits);
            }

            #[test]
            fn t_chgdet_06_created_file_is_reported_when_detector_emits() {
                if $emits {
                    c::check_emits_path(Arc::new($make));
                }
            }

            #[test]
            fn t_chgdet_07_messages_keep_their_folder_stay_below_the_root_and_silent_detectors_stay_silent() {
                c::check_message_semantics(Arc::new($make), $emits);
            }
        }
    };
}

detector_contract_tests!(os_events, OsEvents, emits_events = true);
detector_contract_tests!(scan_only, ScanOnly, emits_events = false);
detector_contract_tests!(
    polling,
    Polling::for_tests(None, Duration::from_millis(50), IoGuardConfig::default()),
    emits_events = true
);
