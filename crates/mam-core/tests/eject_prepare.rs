//! USB 디스크 "꺼내기 준비" (T-eject-01 ~ 13).
//!
//! 분류: Happy (01, 07, 09) / Boundary (04 멱등, 10 이미 붙들린 디스크에 폴더 추가) / Validation (02 네트워크, 03 오프라인, 05 없는 폴더) /
//! Business rule (01 감시·점검, 06 작업 취소·다른 디스크 보호, 08 해제·재연결, 11 훅) / Concurrency (06 실행 중 작업) /
//! External (12 실제 사이드카 프로세스 종료, 13 핸들 해제).
//! 실제 SQLite + 가짜 볼륨(임시 폴더) + 수동 시계. 스레드 대기는 채널·상한 있는 `wait_until` (조건 대기 전용).
#![cfg(windows)]

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::{Env, wait_until};
use mam_core::VolumeKind;
use mam_core::jobs::{Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, NewJob};
use mam_core::scan::{Availability, IngestError};

const WAIT: Duration = Duration::from_secs(30);

/// 두 번째(이동식) 볼륨 `usb` 를 연결한다 — 폴더 `lib` 하나가 있다.
fn plug_usb(env: &Env) -> PathBuf {
    let dir = env.vol.parent().unwrap().join("usb");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib").join("u.jpg"), [7u8; 90]).unwrap();
    env.volumes.add("usb", VolumeKind::Removable, &dir);
    dir
}

/// vol-a 에 폴더 2개(`lib1`, `lib2`) + usb 에 폴더 1개. 반환: (vol-a 폴더들, usb 폴더)
fn three_folders(env: &Env) -> (Vec<i64>, i64) {
    env.write("lib1/a.jpg", 1);
    env.write("lib2/b.jpg", 2);
    let usb = plug_usb(env);
    let a1 = env
        .ingest
        .add_folder(env.path("lib1").to_str().unwrap())
        .unwrap()
        .folder_id;
    let a2 = env
        .ingest
        .add_folder(env.path("lib2").to_str().unwrap())
        .unwrap()
        .folder_id;
    let u = env
        .ingest
        .add_folder(usb.join("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    (vec![a1, a2], u)
}

fn view(env: &Env, id: i64) -> mam_core::scan::ManagedFolderView {
    env.ingest
        .list_folders()
        .unwrap()
        .into_iter()
        .find(|f| f.folder_id == id)
        .unwrap()
}

#[test]
fn t_eject_01_prepare_stops_every_watch_on_the_volume_and_supervise_never_restarts_them() {
    // Arrange: 같은 디스크에 폴더 2개, 다른 디스크에 1개 - 모두 감시 중
    let env = Env::new();
    let (a, u) = three_folders(&env);
    assert!(a.iter().chain([&u]).all(|id| env.ingest.is_watching(*id)));

    // Act
    let res = env.ingest.prepare_volume_eject(a[0]).unwrap();
    let report1 = env.ingest.supervise_once().unwrap();
    let report2 = env.ingest.supervise_once().unwrap();

    // Assert: 같은 디스크의 폴더 전부 중지, 다른 디스크는 그대로
    assert_eq!(res.folder_ids, a);
    assert!(a.iter().all(|id| !env.ingest.is_watching(*id)));
    assert!(env.ingest.is_watching(u));
    // 점검 두 번이 지나도 온라인 전환·감시 재시작·재스캔 예약이 없다
    for r in [&report1, &report2] {
        assert!(
            r.went_online.is_empty() && r.scheduled.iter().all(|f| *f == u),
            "{r:?}"
        );
    }
    assert!(a.iter().all(|id| !env.ingest.is_watching(*id)));
    // 목록: 오프라인 + 준비 표시 (다른 디스크는 아님)
    for id in &a {
        let v = view(&env, *id);
        assert!(v.eject_prepared);
        assert_eq!(v.availability, Availability::Offline);
    }
    let other = view(&env, u);
    assert!(!other.eject_prepared);
    assert_eq!(other.availability, Availability::Online);
    // 반환 마운트 지점은 사용자 표기
    assert_eq!(
        res.mount_point,
        mam_core::volume::strip_verbatim(&env.vol.to_string_lossy())
    );
}

#[test]
fn t_eject_02_network_folder_is_rejected_as_invalid_input() {
    // Arrange
    let env = Env::new();
    let net = env.vol.parent().unwrap().join("net");
    fs::create_dir_all(net.join("lib")).unwrap();
    env.volumes.add("nas", VolumeKind::Network, &net);
    let id = env
        .ingest
        .add_folder(net.join("lib").to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act
    let err = env.ingest.prepare_volume_eject(id).unwrap_err();

    // Assert: 아무것도 바뀌지 않았다
    assert!(matches!(err, IngestError::InvalidInput(_)), "{err:?}");
    assert!(env.ingest.ejected_volumes().is_empty());
    assert_eq!(view(&env, id).availability, Availability::Online);
}

#[test]
fn t_eject_03_offline_volume_is_rejected_as_offline() {
    // Arrange: usb 가 분리됐다
    let env = Env::new();
    let (_, u) = three_folders(&env);
    env.volumes.set_connected("usb", false);
    env.ingest.supervise_once().unwrap();

    // Act
    let err = env.ingest.prepare_volume_eject(u).unwrap_err();

    // Assert
    assert!(matches!(err, IngestError::Offline), "{err:?}");
    assert!(env.ingest.ejected_volumes().is_empty());
}

#[test]
fn t_eject_04_prepare_is_idempotent_and_resume_without_prepare_is_a_no_op() {
    // Arrange
    let env = Env::new();
    let (a, u) = three_folders(&env);
    env.ingest.resume_volume_watch(a[0]).unwrap(); // 준비 전 해제 = 아무 일 없음
    assert!(env.ingest.is_watching(a[0]));

    // Act
    let first = env.ingest.prepare_volume_eject(a[1]).unwrap();
    let second = env.ingest.prepare_volume_eject(a[0]).unwrap();

    // Assert
    assert_eq!(first, second);
    assert_eq!(env.ingest.ejected_volumes().len(), 1);
    assert!(
        env.ingest.resume_volume_watch(u).is_ok(),
        "other volume: not prepared -> no-op"
    );
    assert!(
        !env.ingest.is_watching(a[0]),
        "an unrelated resume does not release the hold"
    );
}

#[test]
fn t_eject_05_unknown_folder_is_not_found() {
    let env = Env::new();

    assert!(matches!(
        env.ingest.prepare_volume_eject(999).unwrap_err(),
        IngestError::NotFound
    ));
    assert!(matches!(
        env.ingest.resume_volume_watch(999).unwrap_err(),
        IngestError::NotFound
    ));
}

/// 메타(vol-a 자산)·썸네일(usb 자산) 종류에 꽂는 대역 실행기: 자산별로 취소 신호를 볼 때까지 돈다.
struct Blocker {
    entered: Sender<i64>,
    /// 취소(`should_stop`)를 본 자산
    stopped: Mutex<Vec<i64>>,
    runs: AtomicUsize,
    /// 다른 디스크 자산이 계속 돌게 두는 신호 (테스트 끝에서 켠다)
    release: Arc<AtomicBool>,
    /// 이 자산은 첫 실행 뒤 다시 잡혀도 오래 막지 않는다 (취소된 작업이 대기열에서 다시 도는 것을 흉내)
    ejected_asset: Mutex<Option<i64>>,
}

impl JobExecutor for Blocker {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let asset = job.asset_id.unwrap();
        let first = self.runs.fetch_add(1, Ordering::SeqCst) < 2;
        if !first && *self.ejected_asset.lock().unwrap() == Some(asset) {
            std::thread::sleep(Duration::from_millis(50));
            return Ok(JobOutcome::Yielded);
        }
        let _ = self.entered.send(asset);
        while !self.release.load(Ordering::SeqCst) {
            if ctx.should_stop() {
                self.stopped.lock().unwrap().push(asset);
                return Ok(JobOutcome::Yielded);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(JobOutcome::Done)
    }
}

fn asset_of(env: &Env, name: &str) -> i64 {
    env.count(&format!(
        "SELECT asset_id FROM file_locations WHERE rel_path LIKE '%{name}' AND detached_at IS NULL"
    ))
}

#[test]
fn t_eject_06_running_job_on_the_volume_is_cancelled_and_stays_queued_while_other_volumes_keep_running()
 {
    // Arrange: 두 디스크의 자산마다 작업 1개 (종류가 달라 동시에 돈다)
    let env = Env::new();
    let (tx, rx) = channel();
    let release = Arc::new(AtomicBool::new(false));
    let blocker = Arc::new(Blocker {
        entered: tx,
        stopped: Mutex::default(),
        runs: AtomicUsize::new(0),
        release: Arc::clone(&release),
        ejected_asset: Mutex::default(),
    });
    env.ingest.register_executor(JobKind::Meta, blocker.clone());
    env.ingest
        .register_executor(JobKind::Thumb, blocker.clone());
    let (a, u) = three_folders(&env);
    env.ingest.start().unwrap();
    assert!(
        wait_until(WAIT, || env.count("SELECT COUNT(*) FROM media_assets") == 3),
        "scan finished"
    );
    let (on_a, on_usb) = (asset_of(&env, "a.jpg"), asset_of(&env, "u.jpg"));
    *blocker.ejected_asset.lock().unwrap() = Some(on_a);
    env.ingest
        .jobs()
        .enqueue(NewJob::for_asset(JobKind::Meta, on_a))
        .unwrap();
    env.ingest
        .jobs()
        .enqueue(NewJob::for_asset(JobKind::Thumb, on_usb))
        .unwrap();
    let mut entered = vec![
        rx.recv_timeout(WAIT).unwrap(),
        rx.recv_timeout(WAIT).unwrap(),
    ];
    entered.sort_unstable();
    assert_eq!(entered, {
        let mut e = vec![on_a, on_usb];
        e.sort_unstable();
        e
    });

    // Act
    let started = std::time::Instant::now();
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    let took = started.elapsed();

    // Assert: vol-a 의 작업만 취소 신호를 봤고, 반환 시점에 이미 실행이 끝나 있다
    assert_eq!(*blocker.stopped.lock().unwrap(), vec![on_a]);
    assert!(took < Duration::from_secs(10), "{took:?}");
    // 취소는 실패·시도 횟수가 아니다 - 작업 행은 남는다
    let meta = env.count(&format!(
        "SELECT COUNT(*) FROM jobs WHERE kind='meta' AND asset_id={on_a} AND attempts = 0 AND status IN ('pending','running')"
    ));
    assert_eq!(meta, 1, "cancelled job stays queued without a failure");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        0
    );
    // 다른 디스크의 작업은 그대로 실행 중이다
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM jobs WHERE kind='thumb' AND asset_id={on_usb} AND status='running'")), 1);
    assert!(
        env.ingest
            .jobs()
            .running_jobs()
            .iter()
            .any(|j| j.asset_id == Some(on_usb))
    );
    let _ = u;
    release.store(true, Ordering::SeqCst);
    env.ingest.shutdown();
}

#[test]
fn t_eject_07_reconnect_after_real_removal_clears_the_hold_and_catches_up_with_a_rescan() {
    // Arrange
    let env = Env::new();
    let (a, _) = three_folders(&env);
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    env.ingest.supervise_once().unwrap();
    assert!(
        !env.ingest.ejected_volumes().is_empty(),
        "still held while the device is physically present"
    );

    // Act 1: 장치가 실제로 빠졌다
    env.volumes.set_connected("vol-a", false);
    let gone = env.ingest.supervise_once().unwrap();

    // Assert 1: 붙들기는 풀리고 오프라인은 그대로
    assert!(env.ingest.ejected_volumes().is_empty());
    assert!(!view(&env, a[0]).eject_prepared);
    assert_eq!(view(&env, a[0]).availability, Availability::Offline);
    assert!(gone.went_online.is_empty());

    // Act 2: 다시 꽂았다
    env.volumes.set_connected("vol-a", true);
    let back = env.ingest.supervise_once().unwrap();

    // Assert 2: 일반 재연결 처리 (온라인 + 감시 + 따라잡기 재스캔)
    assert_eq!(back.went_online.len(), 1);
    // 따라잡기 재스캔이 대기열에 있다 (추가 때 예약된 작업이 아직 대기 중이면 중복 예약은 하지 않는다)
    for id in &a {
        assert!(
            env.count(&format!(
                "SELECT COUNT(*) FROM jobs WHERE kind='scan' AND folder_id = {id}"
            )) >= 1
        );
    }
    assert!(a.iter().all(|id| env.ingest.is_watching(*id)));
    assert_eq!(view(&env, a[1]).availability, Availability::Online);
}

#[test]
fn t_eject_08_resume_releases_the_hold_and_runs_the_reconnect_handling() {
    // Arrange
    let env = Env::new();
    let (a, u) = three_folders(&env);
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    env.ingest.supervise_once().unwrap();

    // Act
    env.ingest.resume_volume_watch(a[1]).unwrap();

    // Assert
    assert!(env.ingest.ejected_volumes().is_empty());
    for id in &a {
        let v = view(&env, *id);
        assert!(!v.eject_prepared);
        assert_eq!(v.availability, Availability::Online);
        assert!(env.ingest.is_watching(*id));
    }
    assert!(env.ingest.is_watching(u));
    // 따라잡기 재스캔이 대기열에 있다
    assert!(
        env.count(&format!(
            "SELECT COUNT(*) FROM jobs WHERE kind='scan' AND folder_id = {}",
            a[0]
        )) >= 1
    );
    // 다시 준비할 수 있다
    assert_eq!(env.ingest.prepare_volume_eject(a[0]).unwrap().folder_ids, a);
}

#[test]
fn t_eject_09_prepare_does_not_touch_the_original_files() {
    // Arrange
    let env = Env::new();
    let (a, _) = three_folders(&env);
    let before = common::env::snapshot(&env.vol);

    // Act
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    env.ingest.supervise_once().unwrap();
    env.ingest.resume_volume_watch(a[0]).unwrap();

    // Assert: 원본 보호 회귀 가드
    assert_eq!(common::env::snapshot(&env.vol), before);
}

#[test]
fn t_eject_10_adding_a_folder_on_a_held_volume_is_refused_until_resume() {
    // Arrange
    let env = Env::new();
    let (a, _) = three_folders(&env);
    env.write("lib3/c.jpg", 3);
    env.ingest.prepare_volume_eject(a[0]).unwrap();

    // Act
    let err = env
        .ingest
        .add_folder(env.path("lib3").to_str().unwrap())
        .unwrap_err();

    // Assert: 등록이 볼륨을 온라인으로 되돌리거나 감시를 시작하지 않는다
    assert!(matches!(err, IngestError::Offline), "{err:?}");
    assert_eq!(view(&env, a[0]).availability, Availability::Offline);
    assert!(!env.ingest.is_watching(a[0]));
    env.ingest.resume_volume_watch(a[0]).unwrap();
    assert!(
        env.ingest
            .add_folder(env.path("lib3").to_str().unwrap())
            .is_ok()
    );
}

#[test]
fn t_eject_11_hook_sees_the_volume_and_can_tell_which_assets_are_on_it() {
    // Arrange
    let env = Env::new();
    let (a, u) = three_folders(&env);
    env.ingest.start().unwrap();
    assert!(wait_until(WAIT, || env
        .count("SELECT COUNT(*) FROM media_assets")
        == 3));
    let (on_a, on_usb) = (asset_of(&env, "a.jpg"), asset_of(&env, "u.jpg"));
    let seen: Arc<Mutex<Vec<(String, bool, bool)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    env.ingest.set_eject_hook(Arc::new(move |scope| {
        sink.lock().unwrap().push((
            scope.mount_point.clone(),
            scope.contains_asset(on_a),
            scope.contains_asset(on_usb),
        ));
    }));

    // Act
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    let _ = u;

    // Assert
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].1 && !seen[0].2, "{seen:?}");
    env.ingest.shutdown();
}

#[test]
fn t_eject_12_gate_kills_the_real_sidecar_process_tree_and_blocks_new_ones() {
    use mam_media::tool::{CancellableProcessRunner, CommandRunner, RunOptions, Tool};
    use mam_media::{MediaError, SidecarGate};

    // Arrange: `.cmd` 대역 ffmpeg (ping 으로 30초 동안 도는 자식 프로세스 트리) - 인자에 E: 같은 볼륨 경로가 실린다
    let env = Env::new();
    let (a, _) = three_folders(&env);
    let gate = SidecarGate::new();
    env.ingest.set_sidecar_gate(Arc::clone(&gate));
    let script = env.data.join("fake-ffmpeg.cmd");
    fs::write(&script, "@echo off\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();
    let runner = CancellableProcessRunner::new(env.ingest.jobs().shutdown_handle())
        .with_gate(Arc::clone(&gate));
    let target = env.path("lib1/a.jpg");
    let args = vec!["-i".into(), target.clone().into_os_string()];
    let opts = RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 1 << 20,
    };
    let worker = {
        let (script, args) = (script.clone(), args.clone());
        std::thread::spawn(move || runner.run(Tool::Ffmpeg, &script, &args, &opts))
    };
    assert!(
        wait_until(WAIT, || gate.running_under(env.vol.to_str().unwrap()) == 1),
        "sidecar running"
    );

    // Act
    let started = std::time::Instant::now();
    env.ingest.prepare_volume_eject(a[0]).unwrap();
    let took = started.elapsed();
    let result = worker.join().unwrap();

    // Assert: 30초 ping 을 기다리지 않고 트리가 죽었으며, 등록이 비었다
    assert!(
        matches!(result, Err(MediaError::Cancelled { .. })),
        "{result:?}"
    );
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(gate.running_under(env.vol.to_str().unwrap()), 0);
    // 붙들려 있는 동안 같은 경로의 새 실행은 띄우지 않는다
    let again = CancellableProcessRunner::new(env.ingest.jobs().shutdown_handle())
        .with_gate(Arc::clone(&gate));
    let err = again
        .run(
            Tool::Ffmpeg,
            &script,
            &args,
            &RunOptions {
                timeout: Duration::from_secs(60),
                max_stdout: 1 << 20,
            },
        )
        .unwrap_err();
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    // 해제하면 다시 허용된다
    env.ingest.resume_volume_watch(a[0]).unwrap();
    let ok = env.data.join("ok.cmd");
    fs::write(
        &ok,
        "@echo off
exit /b 3
",
    )
    .unwrap();
    let out = again
        .run(
            Tool::Ffmpeg,
            &ok,
            &args,
            &RunOptions {
                timeout: Duration::from_secs(60),
                max_stdout: 1 << 20,
            },
        )
        .unwrap();
    assert_eq!(out.code, Some(3));
}

/// 디렉터리를 **독점 공유 모드(share 0)** 로 열 수 있는가 - 다른 핸들이 하나라도 열려 있으면 공유 위반으로 실패한다
/// (Windows 꺼내기가 볼륨을 잠글 수 있는 조건과 같은 종류의 검사. 삭제·이름 변경은 감시 핸들이 FILE_SHARE_DELETE 라 막지 못한다).
fn can_open_exclusively(dir: &std::path::Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .is_ok()
}

#[test]
fn t_eject_13_no_directory_handle_remains_open_under_the_volume_after_prepare() {
    // Arrange: 감시 중에는 독점으로 열 수 없다 (대조군 T-eject-14 와 같은 전제)
    let env = Env::new();
    let (a, u) = three_folders(&env);
    let usb_lib = env.vol.parent().unwrap().join("usb").join("lib");
    assert!(!can_open_exclusively(&env.path("lib1")));

    // Act: prepare 가 반환한 **직후** 검사한다 (감시 스레드가 핸들을 닫을 때까지 기다린 뒤 돌아와야 한다)
    env.ingest.prepare_volume_eject(a[0]).unwrap();

    // Assert: 그 디스크의 폴더는 모두 독점으로 열린다 - 남은 핸들 없음. 다른 디스크의 감시는 그대로
    assert!(
        can_open_exclusively(&env.path("lib1")),
        "watch handle still open on lib1"
    );
    assert!(
        can_open_exclusively(&env.path("lib2")),
        "watch handle still open on lib2"
    );
    assert!(env.ingest.is_watching(u));
    assert!(
        !can_open_exclusively(&usb_lib),
        "other volume keeps its watch"
    );
}

#[test]
fn t_eject_14_control_a_watched_directory_cannot_be_opened_exclusively_without_prepare() {
    // Arrange: T-eject-13 의 대조군 - 준비 없이 감시 중이면 핸들이 열려 있어 독점 열기가 실패한다
    let env = Env::new();
    let (a, _) = three_folders(&env);
    assert!(env.ingest.is_watching(a[0]));

    // Act / Assert
    assert!(
        !can_open_exclusively(&env.path("lib1")),
        "the check must detect an open watch handle"
    );
    env.ingest.resume_volume_watch(a[0]).unwrap(); // 준비하지 않은 해제는 아무것도 바꾸지 않는다
    assert!(!can_open_exclusively(&env.path("lib1")));
}

// ---- 꺼낼 수 있는 디스크 판정 (`ejectable`) ----

fn ejectable_of(env: &Env, id: i64) -> bool {
    view(env, id).ejectable
}

fn folder_on(env: &Env, key_dir: &str) -> i64 {
    env.write(&format!("{key_dir}/x.jpg"), 5);
    env.ingest
        .add_folder(env.path(key_dir).to_str().unwrap())
        .unwrap()
        .folder_id
}

#[test]
fn t_eject_15_usb_bus_is_ejectable_and_nvme_internal_is_not_and_prepare_refuses_it() {
    // Arrange: usb(버스 USB), vol-a(NVMe 내장)
    let env = Env::new();
    env.volumes.set_ejectable("vol-a", Some(false));
    env.volumes.set_ejectable("usb", Some(true));
    let (a, u) = three_folders(&env);

    // Act
    let err = env.ingest.prepare_volume_eject(a[0]).unwrap_err();

    // Assert
    assert!(ejectable_of(&env, u));
    assert!(!ejectable_of(&env, a[0]));
    assert!(matches!(err, IngestError::InvalidInput(_)), "{err:?}");
    assert!(env.ingest.ejected_volumes().is_empty());
    assert!(env.ingest.is_watching(a[0]), "nothing was stopped");
    assert_eq!(view(&env, a[0]).availability, Availability::Online);
    assert!(env.ingest.prepare_volume_eject(u).is_ok());
}

#[test]
fn t_eject_16_device_query_failure_is_treated_as_ejectable() {
    // Arrange
    let env = Env::new();
    env.volumes.set_ejectable("vol-a", None);
    let id = folder_on(&env, "lib");

    // Act / Assert: 목록 표시도 prepare 도 허용
    assert!(ejectable_of(&env, id));
    assert!(env.ingest.prepare_volume_eject(id).is_ok());
}

#[test]
fn t_eject_17_system_volume_is_never_ejectable_even_on_a_usb_bus() {
    // Arrange: %SystemRoot% 가 vol-a 에 있다
    let env = Env::new();
    env.volumes.set_ejectable("vol-a", Some(true));
    env.ingest.set_system_root(env.path("Windows"));
    let a = folder_on(&env, "lib");
    let u = three_usb_folder(&env);

    // Act / Assert
    assert!(!ejectable_of(&env, a));
    assert!(ejectable_of(&env, u));
    assert!(matches!(
        env.ingest.prepare_volume_eject(a).unwrap_err(),
        IngestError::InvalidInput(_)
    ));
}

fn three_usb_folder(env: &Env) -> i64 {
    let usb = plug_usb(env);
    env.ingest
        .add_folder(usb.join("lib").to_str().unwrap())
        .unwrap()
        .folder_id
}

#[test]
fn t_eject_18_volume_holding_mamdata_or_the_backup_dir_is_not_ejectable() {
    // Arrange: MAMData 폴더가 vol-a 안에 있다 (백업 폴더도 같은 방식으로 auto_excludes)
    let env = Env::with_config(|cfg, vol| cfg.auto_excludes.push(vol.join("MAMData2")));
    let a = folder_on(&env, "lib");
    let u = three_usb_folder(&env);

    // Act / Assert
    assert!(!ejectable_of(&env, a));
    assert!(ejectable_of(&env, u));
    assert!(matches!(
        env.ingest.prepare_volume_eject(a).unwrap_err(),
        IngestError::InvalidInput(_)
    ));
}

#[test]
fn t_eject_19_network_volume_is_never_ejectable() {
    // Arrange
    let env = Env::new();
    let net = env.vol.parent().unwrap().join("net");
    fs::create_dir_all(net.join("lib")).unwrap();
    env.volumes.add("nas", VolumeKind::Network, &net);
    let id = env
        .ingest
        .add_folder(net.join("lib").to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act / Assert
    assert!(!ejectable_of(&env, id));
}

#[test]
fn t_eject_20_a_prepared_volume_stays_ejectable_so_the_resume_menu_remains() {
    // Arrange
    let env = Env::new();
    let (a, _) = three_folders(&env);
    env.ingest.prepare_volume_eject(a[0]).unwrap();

    // Act / Assert
    assert!(ejectable_of(&env, a[0]));
}
