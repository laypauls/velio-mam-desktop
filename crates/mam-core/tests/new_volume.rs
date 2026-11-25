//! 새 디스크 감지·대기 목록·"나중에"/기억·다시 묻기·장치 알림 깨우기 통합 테스트 (T-newvol-01 ~ 16).
//!
//! 분류: Happy (01, 07, 14) / Boundary (02 시작 시 연결, 10 모르는 키, 12 깨진 JSON) / Business rule (03~06, 08, 09, 11) /
//! External (15 감지기 시작 실패) / Concurrency (13 알림 묶음·깨우기, 16 종료 시 감지기 정리).
//! 실제 SQLite + 가짜 볼륨 + 수동 시계. 스레드 대기는 채널 `recv_timeout` (폴링 없음).
#![cfg(windows)]

mod common;

use std::any::Any;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::Env;
use mam_core::device_watch::{DeviceNotify, DeviceWatcher};
use mam_core::ingest::PendingVolume;
use mam_core::{CoreError, VolumeKind};
use mam_db::Priority;

type Events = Arc<Mutex<Vec<Vec<PendingVolume>>>>;

const WAIT: Duration = Duration::from_secs(20);

/// 점검을 한 번 돌려 baseline 을 만든 환경 (vol-a 는 시작 때 이미 연결). 이후 이벤트만 `events` 에 쌓인다.
fn setup() -> (Env, Events) {
    let env = Env::new();
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    env.ingest
        .set_pending_listener(Arc::new(move |v| sink.lock().unwrap().push(v)));
    env.ingest.supervise_once().unwrap();
    assert!(events.lock().unwrap().is_empty(), "nothing is new at start");
    (env, events)
}

/// 새 볼륨(임시 디렉터리 = 볼륨 루트)을 연결한다. `media/a.jpg` 가 들어 있다.
fn plug(env: &Env, key: &str, kind: VolumeKind) -> PathBuf {
    let dir = env.vol.parent().unwrap().join(key);
    fs::create_dir_all(dir.join("media")).unwrap();
    fs::write(dir.join("media").join("a.jpg"), [9u8; 80]).unwrap();
    env.volumes.add(key, kind, &dir);
    dir
}

fn keys(env: &Env) -> Vec<String> {
    env.ingest
        .pending_volumes()
        .into_iter()
        .map(|p| p.volume_key)
        .collect()
}

fn volume_rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM volumes")
}

/// 분리 -> 점검 -> 다시 연결 -> 점검
fn replug(env: &Env, key: &str) {
    env.volumes.set_connected(key, false);
    env.ingest.supervise_once().unwrap();
    env.volumes.set_connected(key, true);
    env.ingest.supervise_once().unwrap();
}

#[test]
fn t_newvol_01_volume_appearing_after_start_becomes_pending_with_one_event() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.volumes.set_total_bytes("usb", 62_000_000_000);

    // Act
    env.ingest.supervise_once().unwrap();
    env.ingest.supervise_once().unwrap(); // 같은 상태 반복은 이벤트 없음

    // Assert
    let pending = env.ingest.pending_volumes();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].volume_key, "usb");
    assert_eq!(pending[0].kind, VolumeKind::Removable);
    assert_eq!(pending[0].total_bytes, Some(62_000_000_000));
    assert_eq!(pending[0].label.as_deref(), Some("usb"));
    assert_eq!(events.lock().unwrap().as_slice(), [pending]);
    assert_eq!(
        volume_rows(&env),
        0,
        "pending disks never enter the volumes table"
    );
}

#[test]
fn t_newvol_02_volume_already_connected_at_start_is_never_asked() {
    // Arrange: 첫 점검 전에 이미 연결된 외장 HDD (kind=fixed)
    let env = Env::new();
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    env.ingest
        .set_pending_listener(Arc::new(move |v| sink.lock().unwrap().push(v)));
    plug(&env, "hdd", VolumeKind::Fixed);

    // Act
    env.ingest.supervise_once().unwrap();
    env.ingest.supervise_once().unwrap();

    // Assert
    assert!(env.ingest.pending_volumes().is_empty());
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn t_newvol_03_registered_volume_reconnect_keeps_the_existing_online_path() {
    // Arrange: 관리 폴더가 있는 vol-a 를 분리했다가 다시 연결 (회귀)
    let (env, events) = setup();
    env.write("lib/a.jpg", 1);
    env.ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap();
    env.volumes.set_connected("vol-a", false);
    let off = env.ingest.supervise_once().unwrap();
    env.volumes.set_connected("vol-a", true);

    // Act
    let on = env.ingest.supervise_once().unwrap();

    // Assert: 기존 재연결 경로(오프라인 -> 온라인 + 재스캔 예약)만 탄다, 새 디스크 질문 없음
    assert_eq!(off.went_offline.len(), 1);
    assert_eq!(on.went_online, off.went_offline);
    assert!(env.ingest.pending_volumes().is_empty());
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn t_newvol_04_later_without_remember_hides_only_this_connection() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();

    // Act: 기억하지 않고 나중에
    env.ingest.dismiss_new_volume("usb", false).unwrap();

    // Assert: 이번 연결에서는 다시 묻지 않는다
    assert!(env.ingest.pending_volumes().is_empty());
    env.ingest.supervise_once().unwrap();
    assert!(env.ingest.pending_volumes().is_empty());
    assert!(env.ingest.list_ignored_volumes().unwrap().is_empty());
    assert_eq!(events.lock().unwrap().len(), 2, "added, then removed");

    // 분리 후 다시 연결하면 다시 묻는다
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();
    env.volumes.set_connected("usb", true);
    env.ingest.supervise_once().unwrap();
    assert_eq!(keys(&env), ["usb"]);
    assert_eq!(volume_rows(&env), 0);
}

#[test]
fn t_newvol_05_remembered_disk_is_not_asked_again_and_causes_no_backfill_or_volume_row() {
    // Arrange
    let (env, _events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();
    env.clock.advance(1_234);

    // Act
    env.ingest.dismiss_new_volume("usb", true).unwrap();
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();
    env.volumes.set_connected("usb", true);
    let report = env.ingest.supervise_once().unwrap();

    // Assert
    let ignored = env.ingest.list_ignored_volumes().unwrap();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].volume_key, "usb");
    assert_eq!(ignored[0].ignored_at, env.clock_now());
    assert_eq!(ignored[0].label.as_deref(), Some("usb"));
    assert!(env.ingest.pending_volumes().is_empty());
    assert!(report.went_online.is_empty(), "not a managed volume");
    assert!(report.backfill.is_none(), "no job backfill on plug-in");
    assert_eq!(volume_rows(&env), 0);
}

#[test]
fn t_newvol_06_connected_ignored_disk_only_refreshes_label_and_mount_point() {
    // Arrange
    let (env, _events) = setup();
    let dir = plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();
    env.ingest.dismiss_new_volume("usb", true).unwrap();
    let moved = dir.parent().unwrap().join("usb_moved");
    fs::create_dir_all(&moved).unwrap();

    // Act: 드라이브 문자·라벨이 바뀐 채 다시 연결
    env.volumes.remount("usb", &moved);
    env.volumes.set_label("usb", "CAMERA");
    env.ingest.supervise_once().unwrap();

    // Assert
    let ignored = env.ingest.list_ignored_volumes().unwrap();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].label.as_deref(), Some("CAMERA"));
    assert_eq!(
        ignored[0].last_mount_point.as_deref(),
        Some(moved.to_str().unwrap().trim_start_matches(r"\\?\"))
    );
    assert!(env.ingest.pending_volumes().is_empty());
}

#[test]
fn t_newvol_07_two_partitions_plugged_together_are_one_event_with_two_cards() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "p1", VolumeKind::Removable);
    plug(&env, "p2", VolumeKind::Removable);

    // Act
    env.ingest.supervise_once().unwrap();
    env.ingest.dismiss_new_volume("p1", false).unwrap();

    // Assert: 카드별 처리 — p1 만 빠진다
    let ev = events.lock().unwrap();
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0].len(), 2);
    assert_eq!(
        ev[1]
            .iter()
            .map(|p| p.volume_key.as_str())
            .collect::<Vec<_>>(),
        ["p2"]
    );
}

#[test]
fn t_newvol_08_drive_letter_change_of_a_pending_disk_updates_the_card() {
    // Arrange
    let (env, events) = setup();
    let dir = plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();
    let moved = dir.parent().unwrap().join("usb_g");
    fs::create_dir_all(&moved).unwrap();

    // Act
    env.volumes.remount("usb", &moved);
    env.ingest.supervise_once().unwrap();

    // Assert: 같은 키, 새 마운트 지점으로 목록 갱신
    let pending = env.ingest.pending_volumes();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].mount_point,
        moved.to_str().unwrap().trim_start_matches(r"\\?\")
    );
    assert_eq!(events.lock().unwrap().len(), 2);
}

#[test]
fn t_newvol_09_add_folder_removes_the_card_and_a_fully_removed_disk_is_asked_on_next_connection() {
    // Arrange
    let (env, events) = setup();
    let dir = plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();

    // Act: 등록 성공 -> 대기 목록에서 자동 제거
    let folder = env
        .ingest
        .add_folder(dir.join("media").to_str().unwrap())
        .unwrap();

    // Assert
    assert!(env.ingest.pending_volumes().is_empty());
    assert_eq!(events.lock().unwrap().last().unwrap().len(), 0);

    // 관리 폴더를 모두 지워도 연결된 동안은 다시 묻지 않고, 다음 연결 때 다시 묻는다
    env.ingest.remove_folder(folder.folder_id).unwrap();
    env.ingest.wait_removals();
    env.ingest.supervise_once().unwrap();
    assert!(env.ingest.pending_volumes().is_empty());
    replug(&env, "usb");
    assert_eq!(keys(&env), ["usb"]);
}

#[test]
fn t_newvol_10_dismissing_an_unknown_or_gone_key_is_a_silent_no_op() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();
    let before = events.lock().unwrap().len();

    // Act / Assert: 분리된 디스크·처음 보는 키 — remember=true 여도 저장하지 않는다
    env.ingest.dismiss_new_volume("usb", true).unwrap();
    env.ingest.dismiss_new_volume("nope", true).unwrap();
    env.ingest.dismiss_new_volume("nope", false).unwrap();
    assert!(env.ingest.list_ignored_volumes().unwrap().is_empty());
    assert_eq!(events.lock().unwrap().len(), before, "no event");
}

#[test]
fn t_newvol_11_unignore_of_a_connected_disk_puts_it_back_on_the_pending_list_immediately() {
    // Arrange: 시작 때부터 연결돼 있던(baseline) 디스크를 기억 목록에 올려 둔 상태
    let env = Env::new();
    let events: Events = Arc::default();
    let sink = Arc::clone(&events);
    env.ingest
        .set_pending_listener(Arc::new(move |v| sink.lock().unwrap().push(v)));
    plug(&env, "hdd", VolumeKind::Fixed);
    env.ingest.supervise_once().unwrap();
    env.pool
        .write(Priority::High, |c| {
            mam_db::ignored_volumes::add_ignored_volume(
                c,
                &mam_db::ignored_volumes::IgnoredVolume {
                    volume_key: "hdd".into(),
                    label: None,
                    last_mount_point: None,
                    ignored_at: 5,
                },
            )?;
            Ok(())
        })
        .unwrap();

    // Act: 점검을 기다리지 않고 바로
    env.ingest.unignore_volume("hdd").unwrap();

    // Assert
    assert_eq!(keys(&env), ["hdd"]);
    assert_eq!(events.lock().unwrap().len(), 1);
    assert!(env.ingest.list_ignored_volumes().unwrap().is_empty());
}

#[test]
fn t_newvol_12_unignore_of_a_disconnected_or_unknown_disk_waits_for_reconnection() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();
    env.ingest.dismiss_new_volume("usb", true).unwrap();
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();
    let before = events.lock().unwrap().len();

    // Act
    env.ingest.unignore_volume("usb").unwrap();
    env.ingest.unignore_volume("never-seen").unwrap(); // 목록에 없음 -> 아무 일 없음

    // Assert: 분리 중이면 대기 목록에 오르지 않고, 다시 연결되면 묻는다
    assert!(env.ingest.pending_volumes().is_empty());
    assert_eq!(events.lock().unwrap().len(), before);
    env.volumes.set_connected("usb", true);
    env.ingest.supervise_once().unwrap();
    assert_eq!(keys(&env), ["usb"]);
}

#[test]
fn t_newvol_13_corrupt_ignored_setting_is_treated_as_empty_and_does_not_break_detection() {
    // Arrange
    let (env, _events) = setup();
    env.pool
        .write(Priority::High, |c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES ('ignored_volumes', '{broken')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    plug(&env, "usb", VolumeKind::Removable);

    // Act
    env.ingest.supervise_once().unwrap();

    // Assert
    assert_eq!(keys(&env), ["usb"]);
    assert!(env.ingest.list_ignored_volumes().unwrap().is_empty());
}

#[test]
fn t_newvol_14_disconnected_pending_disk_leaves_the_list() {
    // Arrange
    let (env, events) = setup();
    plug(&env, "usb", VolumeKind::Removable);
    env.ingest.supervise_once().unwrap();

    // Act
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();

    // Assert: 목록이 비고 빈 목록 이벤트가 나간다
    assert!(env.ingest.pending_volumes().is_empty());
    assert!(events.lock().unwrap().last().unwrap().is_empty());
}

/// 가짜 감지기: 테스트가 `trigger` 로 장치 변경 알림을 낸다. 핸들이 drop 되면 `stopped` 가 켜진다.
#[derive(Default)]
struct FakeWatcher {
    notify: Mutex<Option<DeviceNotify>>,
    stopped: Arc<AtomicBool>,
    fail: bool,
}

struct FakeHandle(Arc<AtomicBool>);

impl Drop for FakeHandle {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl DeviceWatcher for FakeWatcher {
    fn start(&self, notify: DeviceNotify) -> Result<Box<dyn Any + Send>, CoreError> {
        if self.fail {
            return Err(CoreError::Volume("fake watcher failure".into()));
        }
        *self.notify.lock().unwrap() = Some(notify);
        Ok(Box::new(FakeHandle(Arc::clone(&self.stopped))))
    }
}

impl FakeWatcher {
    fn trigger(&self) {
        let n = self.notify.lock().unwrap().clone().expect("started");
        n();
    }
}

/// 30초(여기서는 1시간) 주기 대신 장치 알림만으로 점검이 깨어나는 환경.
fn started_env(watcher: Arc<FakeWatcher>) -> (Env, std::sync::mpsc::Receiver<Vec<PendingVolume>>) {
    let env = Env::with_config(|c, _| c.supervise_every = Duration::from_secs(3600));
    let (tx, rx) = channel();
    env.ingest
        .set_pending_listener(Arc::new(move |v| drop(tx.send(v))));
    env.ingest
        .set_device_watcher(watcher, Duration::from_millis(40));
    (env, rx)
}

#[test]
fn t_newvol_15_device_notification_wakes_the_supervisor_long_before_the_periodic_tick() {
    // Arrange: 첫 점검(baseline)이 끝난 것을 list_volumes 호출 신호로 확인한 뒤 디스크를 꽂는다
    let watcher = Arc::new(FakeWatcher::default());
    let (env, rx) = started_env(Arc::clone(&watcher));
    let first_tick = env.volumes.observe();
    env.ingest.start().unwrap();
    first_tick.recv_timeout(WAIT).expect("first supervise ran");
    plug(&env, "usb", VolumeKind::Removable);

    // Act: 알림 3번 (파티션 여러 개) -> 디바운스 후 점검 1회
    for _ in 0..3 {
        watcher.trigger();
    }

    // Assert: 1시간 주기를 기다리지 않고 대기 목록 이벤트가 온다
    let pending = rx
        .recv_timeout(WAIT)
        .expect("woken supervise reported the disk");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].volume_key, "usb");
    env.ingest.shutdown();
}

#[test]
fn t_newvol_16_shutdown_stops_the_device_watcher() {
    // Arrange
    let watcher = Arc::new(FakeWatcher::default());
    let stopped = Arc::clone(&watcher.stopped);
    let (env, _rx) = started_env(Arc::clone(&watcher));
    env.ingest.start().unwrap();
    assert!(!stopped.load(Ordering::SeqCst));

    // Act
    env.ingest.shutdown();

    // Assert
    assert!(stopped.load(Ordering::SeqCst));
}

#[test]
fn t_newvol_17_watcher_start_failure_keeps_the_periodic_supervisor_running() {
    // Arrange
    let watcher = Arc::new(FakeWatcher {
        fail: true,
        ..FakeWatcher::default()
    });
    let (env, _rx) = started_env(watcher);
    let ticks = env.volumes.observe();

    // Act
    env.ingest.start().unwrap();

    // Assert: 감지기 없이도 첫 점검은 돈다
    ticks.recv_timeout(WAIT).expect("supervise still runs");
    env.ingest.shutdown();
}
