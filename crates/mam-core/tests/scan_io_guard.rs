//! 스캔 순회의 제한 시간 (T-ioguard-09 ~ 13, T-lane-08) — 응답 없는 볼륨이 다른 볼륨을 막지 않고, 연결 끊김은 삭제가 아니다.
//!
//! 분류: External (09~12 막힌 파일, 08 막힌 볼륨) / Business rule (09 missing 이 되지 않는다, 11 응답 없음에서 멈추고 이어간다) /
//! Concurrency (10 같은 파일 재검사 없음, T-lane-08 다른 볼륨 스캔 계속) / 원본 보호 (09 픽스처 불변).
//! 막힘은 `FileIdentityProvider` 가짜가 만든다 — 실제 네트워크 공유에 의존하지 않는다. 막힐 수 있는 볼륨 종류(네트워크)로 등록한다.
#![cfg(windows)]

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use common::env::{Env, snapshot, wait_until};
use mam_core::scan::{IoGuardConfig, ScanOutcome, scan_folder};
use mam_core::{CoreError, FileIdentityProvider, FileMeta, VolumeKind, WindowsFileIdentity};

const SAFETY: Duration = Duration::from_secs(30);
/// 막힌 파일이 시간 초과되는 시험의 제한 시간 — 막힌 호출은 풀기 전까지 돌아오지 않으므로 길이와 무관하게 시간 초과된다.
/// 정상 파일의 조회가 부하(병렬 테스트·백신 검사)로 느려져도 넘지 않을 만큼 길게 둔다 (예전 100~120ms 는 24배 동시 실행에서 정상 경로가 시간 초과)
const GUARD_MS: u64 = 1_500;
/// 제한 시간이 시험 대상이 아닌 시험 (내장 디스크는 가드를 거치지 않고, 013 은 상한 1 의 자리 점유로 즉시 실패한다)
const SAFETY_MS: u64 = 30_000;

/// 경로에 `blocked` 문자열이 들어 있는 파일의 `stat` 을 열기 전까지 막는다 ("응답 없는 공유").
struct StuckShare {
    inner: WindowsFileIdentity,
    blocked: Mutex<HashSet<String>>,
    open: Mutex<bool>,
    cv: Condvar,
    /// 막힌 경로로 들어온 `stat` 호출 수 (같은 파일을 중복 검사하지 않는지 확인용)
    entered: AtomicUsize,
}

impl StuckShare {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: WindowsFileIdentity,
            blocked: Mutex::default(),
            open: Mutex::new(false),
            cv: Condvar::new(),
            entered: AtomicUsize::new(0),
        })
    }
    fn block(&self, needle: &str) {
        *self.open.lock().unwrap() = false;
        self.blocked.lock().unwrap().insert(needle.to_owned());
    }
    /// 막힌 호출을 모두 푼다
    fn release(&self) {
        self.blocked.lock().unwrap().clear();
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

impl FileIdentityProvider for StuckShare {
    fn stat(&self, path: &Path) -> Result<FileMeta, CoreError> {
        let p = path.to_string_lossy().into_owned();
        if self.blocked.lock().unwrap().iter().any(|n| p.contains(n.as_str())) {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let g = self.open.lock().unwrap();
            let _ = self.cv.wait_timeout_while(g, SAFETY, |open| !*open).unwrap();
        }
        self.inner.stat(path)
    }
}

fn env_with(share: &Arc<StuckShare>, timeout_ms: u64, max_per_volume: usize) -> Env {
    Env::with_identity(
        move |c, _| {
            c.io_guard = IoGuardConfig {
                timeout: Duration::from_millis(timeout_ms),
                max_per_volume,
            };
        },
        Arc::clone(share) as Arc<dyn FileIdentityProvider + Send + Sync>,
    )
}

/// 네트워크 볼륨 `nas` 에 폴더 `lib` 와 파일들을 만들고 등록한다. (폴더 id, 폴더 경로)
fn nas_folder(env: &Env, files: &[&str]) -> (i64, PathBuf) {
    let root = env.vol.parent().unwrap().join("nas");
    fs::create_dir_all(root.join("lib")).unwrap();
    for (i, f) in files.iter().enumerate() {
        fs::write(root.join("lib").join(f), vec![u8::try_from(i).unwrap() + 1; 64]).unwrap();
    }
    env.volumes.add("nas", VolumeKind::Network, &root);
    let id = env.ingest.add_folder(root.join("lib").to_str().unwrap()).unwrap().folder_id;
    (id, root)
}

fn scan(env: &Env, id: i64) -> ScanOutcome {
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap()
}

fn presence(env: &Env, name: &str) -> String {
    env.read(|c| {
        c.query_row(
            "SELECT presence FROM file_locations WHERE rel_path LIKE ?1 AND detached_at IS NULL",
            [format!("%{name}")],
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn unresolved_unresponsive(env: &Env, rel: &str) -> i64 {
    env.count(&format!(
        "SELECT COUNT(*) FROM scan_errors WHERE error_code = 'unresponsive' AND rel_path = '{rel}' AND resolved_at IS NULL"
    ))
}

#[test]
fn t_ioguard_09_a_file_that_times_out_is_skipped_recorded_and_never_marked_missing() {
    // Arrange: 두 파일을 정상 스캔해 둔 뒤 b.jpg 의 응답이 끊긴다
    let share = StuckShare::new();
    let env = env_with(&share, GUARD_MS, 4);
    let (id, root) = nas_folder(&env, &["a.jpg", "b.jpg"]);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    let before = snapshot(&root);
    let revision = env.count("SELECT SUM(revision) FROM media_assets");
    share.block("b.jpg");
    env.clock.advance(5_000);

    // Act
    let outcome = scan(&env, id);

    // Assert: 스캔은 끝났고, b.jpg 는 missing 이 아니며 "응답 없음" 으로 기록됐다
    assert!(matches!(outcome, ScanOutcome::Completed { .. }), "{outcome:?}");
    assert_eq!(presence(&env, "b.jpg"), "present");
    assert_eq!(presence(&env, "a.jpg"), "present");
    assert_eq!(unresolved_unresponsive(&env, "b.jpg"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"), 0);
    assert_eq!(env.count("SELECT SUM(revision) FROM media_assets"), revision, "no asset was changed");

    // Act 2: 응답이 돌아온 뒤 다시 스캔 — 오류가 해결되고 자산은 그대로
    share.release();
    env.clock.advance(5_000);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    assert_eq!(unresolved_unresponsive(&env, "b.jpg"), 0);
    assert_eq!(presence(&env, "b.jpg"), "present");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);
    assert_eq!(snapshot(&root), before, "originals are untouched");
}

#[test]
fn t_ioguard_10_rescanning_while_a_file_is_still_hung_does_not_start_another_check() {
    // Arrange
    let share = StuckShare::new();
    let env = env_with(&share, GUARD_MS, 4);
    let (id, _) = nas_folder(&env, &["a.jpg", "b.jpg"]);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    share.block("b.jpg");

    // Act: 같은 파일이 막힌 채로 스캔을 세 번 돌린다
    for _ in 0..3 {
        env.clock.advance(5_000);
        assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    }

    // Assert: 막힌 호출은 한 번만 들어갔고 스레드는 하나뿐이다
    assert!(wait_until(Duration::from_secs(5), || share.entered.load(Ordering::SeqCst) >= 1));
    assert_eq!(share.entered.load(Ordering::SeqCst), 1);
    assert_eq!(presence(&env, "b.jpg"), "present");
    share.release();
    assert!(wait_until(Duration::from_secs(5), || env.ingest.unresponsive_volumes().is_empty()));
}

#[test]
fn t_ioguard_11_an_unresponsive_volume_stops_the_scan_without_missing_and_the_next_run_resumes() {
    // Arrange: 파일 3개를 스캔해 둔 뒤 전부 응답이 끊긴다 (볼륨당 검사 상한 2)
    let share = StuckShare::new();
    let env = env_with(&share, GUARD_MS, 2);
    let (id, root) = nas_folder(&env, &["a.jpg", "b.jpg", "c.jpg"]);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    let before = snapshot(&root);
    for f in ["a.jpg", "b.jpg", "c.jpg"] {
        share.block(f);
    }
    env.clock.advance(5_000);

    // Act
    let outcome = scan(&env, id);

    // Assert: 응답 없는 볼륨에서 멈췄고 끝난 것으로 치지 않으므로 아무것도 missing 이 아니다
    assert_eq!(outcome, ScanOutcome::Unresponsive);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'paused'"), 1);
    assert_eq!(env.ingest.unresponsive_volumes().len(), 1);

    // Act 2: 응답이 돌아오면 같은 실행을 이어서 끝낸다
    share.release();
    assert!(wait_until(Duration::from_secs(5), || env.ingest.unresponsive_volumes().is_empty()));
    env.clock.advance(5_000);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_runs WHERE status = 'paused'"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"), 0);
    assert_eq!(snapshot(&root), before);
}

#[test]
fn t_ioguard_12_a_fixed_volume_scans_inline_without_guard_threads_or_timeouts() {
    // Arrange: 내장 디스크(Fixed)는 호출을 그대로 한다 — 파일마다 검사 스레드를 만들지 않는다
    let share = StuckShare::new();
    let env = env_with(&share, SAFETY_MS, 1);
    env.write("lib/a.jpg", 1);
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;

    // Act
    let outcome = scan(&env, id);

    // Assert
    assert!(matches!(outcome, ScanOutcome::Completed { .. }));
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 1);
    assert!(env.ingest.unresponsive_volumes().is_empty());
    assert_eq!(env.ingest.env().io.timeout_count(), 0);
}

#[test]
fn t_lane_08_a_blocked_volume_does_not_stop_another_volumes_scan() {
    // Arrange: nas 의 파일이 막혀 있고(제한 시간 5초 — 스캔이 그동안 붙들려 있다), 로컬 폴더가 따로 있다
    let share = StuckShare::new();
    let env = env_with(&share, 5_000, 8);
    share.block("hung.jpg");
    let (nas, _) = nas_folder(&env, &["hung.jpg"]);
    env.write("lib/a.jpg", 1);
    let local = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    env.ingest.start().unwrap();

    // Act / Assert: nas 스캔이 막혀 있는 동안 로컬 스캔은 끝난다
    let local_done = wait_until(Duration::from_secs(20), || {
        env.count(&format!("SELECT COUNT(*) FROM managed_folders WHERE folder_id = {local} AND last_full_scan_at IS NOT NULL")) == 1
    });
    assert!(local_done, "local scan completes while the nas scan is blocked");
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE rel_path LIKE '%a.jpg'"), 1);
    assert!(
        wait_until(Duration::from_secs(5), || env.ingest.jobs().running_count(mam_core::jobs::JobKind::Scan) == 1),
        "only the nas scan is still running"
    );
    assert_eq!(
        env.count(&format!("SELECT COUNT(*) FROM managed_folders WHERE folder_id = {nas} AND last_full_scan_at IS NOT NULL")),
        0
    );

    // 풀리면 nas 스캔도 끝난다
    share.release();
    assert!(wait_until(Duration::from_secs(20), || {
        env.count(&format!("SELECT COUNT(*) FROM managed_folders WHERE folder_id = {nas} AND last_full_scan_at IS NOT NULL")) == 1
    }));
    env.ingest.shutdown();
}

/// 디렉터리에 속성을 건다 (`attrib` — 임시 디렉터리 안의 픽스처 전용)
fn attrib(dir: &Path, flags: &[&str]) {
    let status = std::process::Command::new("attrib")
        .args(flags)
        .arg(mam_core::volume::strip_verbatim(&dir.to_string_lossy()))
        .status()
        .expect("attrib runs");
    assert!(status.success());
}

#[test]
fn t_ioguard_13_hidden_ancestor_check_of_a_watch_event_goes_through_the_guard() {
    // Arrange: 네트워크 볼륨 폴더 — 숨김+시스템 폴더 아래 파일과 일반 파일
    let share = StuckShare::new();
    let env = env_with(&share, SAFETY_MS, 1);
    let (id, root) = nas_folder(&env, &["a.jpg"]);
    let hs = root.join("lib").join("hs");
    fs::create_dir_all(&hs).unwrap();
    fs::write(hs.join("x.jpg"), [9u8; 64]).unwrap();
    attrib(&hs, &["+h", "+s"]);
    fs::write(root.join("lib").join("c.jpg"), [3u8; 64]).unwrap();
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    let before = snapshot(&root);
    let assets = env.count("SELECT COUNT(*) FROM media_assets");

    // Act / Assert 1: 정상 응답 — 숨김+시스템 폴더 아래는 무시, 일반 파일은 반영 (가드를 거쳐도 결과가 같다)
    let apply = |p: PathBuf| mam_core::scan::apply_path(env.ingest.env(), id, &p).unwrap();
    assert_eq!(apply(hs.join("x.jpg")), mam_core::scan::PathAction::Ignored);
    assert_eq!(apply(root.join("lib").join("c.jpg")), mam_core::scan::PathAction::Reconciled);

    // Act 2: 이 볼륨의 검사 스레드 자리(상한 1)가 막힌 호출 하나로 차 있다 -> 조상 속성 조회도 새 스레드를 걸지 않고 "응답 없음"
    let volume_id: i64 = env.read(|c| c.query_row("SELECT volume_id FROM managed_folders", [], |r| r.get(0)).unwrap());
    // 자리를 잡은 호출은 시계가 아니라 채널로 푼다 (부하로 느려져도 점유가 먼저 끝나지 않게 — 결정적)
    // 점유 호출만 짧은 제한 시간으로 건다 — 채널이 풀리기 전에는 돌아오지 않으므로 시간 초과는 결정적이고, 스레드는 계속 산다
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let held = env.ingest.env().io.guarded("occupy", volume_id, Duration::from_millis(50), move || {
        let _ = release_rx.recv_timeout(SAFETY);
    });
    assert!(matches!(held, mam_core::scan::Guarded::TimedOut));
    assert!(env.ingest.env().io.is_unresponsive(volume_id), "the only slot is held");
    let act = apply(root.join("lib").join("c.jpg"));

    // Assert 2: 건너뛰고 오류로 남길 뿐 아무것도 바꾸지 않는다 (missing 아님), 원본 불변
    assert_eq!(act, mam_core::scan::PathAction::Failed);
    assert_eq!(unresolved_unresponsive(&env, "c.jpg"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), assets);
    assert_eq!(snapshot(&root), before, "originals are untouched");
    drop(release_tx);
}

/// 첫 `stat` 에서 폴더 루트를 치워 버리는 가짜 — 루트 확인을 통과한 직후 공유가 끊기는 순간을 만든다.
struct DropsRootOnStat {
    inner: WindowsFileIdentity,
    /// (루트, 치운 뒤 이름) — 한 번만 치운다
    root: Mutex<Option<(PathBuf, PathBuf)>>,
}

impl FileIdentityProvider for DropsRootOnStat {
    fn stat(&self, path: &Path) -> Result<FileMeta, CoreError> {
        if let Some((from, to)) = self.root.lock().unwrap().take() {
            fs::rename(from, to).unwrap();
        }
        self.inner.stat(path)
    }
}

#[test]
fn t_ioguard_14_a_path_lookup_that_fails_after_the_share_dropped_is_not_marked_missing() {
    // Arrange: 스캔으로 a.jpg 를 등록한 뒤, 감시 반영 중 첫 stat 에서 루트가 사라진다 (루트 확인 통과 이후)
    let fake = Arc::new(DropsRootOnStat { inner: WindowsFileIdentity, root: Mutex::new(None) });
    let env = Env::with_identity(|_, _| {}, Arc::clone(&fake) as Arc<dyn FileIdentityProvider + Send + Sync>);
    let (id, root) = nas_folder(&env, &["a.jpg"]);
    assert!(matches!(scan(&env, id), ScanOutcome::Completed { .. }));
    let before = snapshot(&root);
    let revision = env.count("SELECT SUM(revision) FROM media_assets");
    let lib = root.join("lib");
    let away = root.join("lib_away");
    *fake.root.lock().unwrap() = Some((lib.clone(), away.clone()));

    // Act
    let act = mam_core::scan::apply_path(env.ingest.env(), id, &lib.join("a.jpg")).unwrap();

    // Assert: 루트가 다시 닿지 않으므로 missing 으로 만들지 않고 Failed(나중에 다시 확인)로 남긴다
    assert_eq!(act, mam_core::scan::PathAction::Failed);
    assert_eq!(presence(&env, "a.jpg"), "present");
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"), 0);
    assert_eq!(env.count("SELECT SUM(revision) FROM media_assets"), revision, "no asset was changed");
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_errors WHERE rel_path = 'a.jpg' AND resolved_at IS NULL"), 1);

    // 공유가 돌아오면(원위치) 원본은 그대로이고, 진짜로 사라진 파일만 missing 이 된다
    fs::rename(&away, &lib).unwrap();
    assert_eq!(snapshot(&root), before, "originals are untouched");
    fs::remove_file(lib.join("a.jpg")).unwrap();
    let gone = mam_core::scan::apply_path(env.ingest.env(), id, &lib.join("a.jpg")).unwrap();
    assert_eq!(gone, mam_core::scan::PathAction::Missing(1));
}
