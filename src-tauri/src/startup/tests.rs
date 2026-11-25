// Categories: Happy (09 검증 성공) / Business rule (10 새 스키마, 11 복구 1회, 12 복구 후 DB) /
// External (13 이전 설치기 실행 실패). 실제 SQLite 임시 파일 + 가짜 Launcher.
// inject-* feature 를 켜면 일부가 의도대로 실패한다 — 시험 러너는 feature 없이 실행한다.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use mam_db::Clock;
use tempfile::TempDir;

use super::*;
use crate::update_guard::{MAX_FAILED_STARTS, State};

struct TestClock(Cell<i64>);

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}

#[derive(Default)]
struct FakeLauncher {
    calls: RefCell<Vec<PathBuf>>,
    fail: bool,
}

impl Launcher for FakeLauncher {
    fn launch(&self, installer: &Path) -> std::io::Result<()> {
        self.calls.borrow_mut().push(installer.to_path_buf());
        if self.fail {
            Err(std::io::Error::other("cannot run installer"))
        } else {
            Ok(())
        }
    }
}

const T0: i64 = 1_700_000_000_000;

fn clock() -> TestClock {
    TestClock(Cell::new(T0))
}

fn setup() -> (TempDir, Paths) {
    let dir = TempDir::new().unwrap();
    let paths = Paths::new(dir.path().join("MAMData"));
    (dir, paths)
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn applied(paths: &Paths) -> Vec<i64> {
    let conn = mam_db::open(&paths.db()).unwrap();
    let mut stmt = conn
        .prepare("SELECT version FROM refinery_schema_history ORDER BY version")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// 앱이 아는 것보다 한 단계 새로운 스키마를 가진 DB 를 만든다 (새 버전이 적용한 것처럼).
/// 반환: 새 스키마 적용 전 스냅샷(= 업데이트 직전 백업) 경로.
fn make_db_newer_than_app(paths: &Paths) -> PathBuf {
    assert_eq!(
        boot(
            &args(&[]),
            "0.1.0",
            paths,
            &clock(),
            &FakeLauncher::default()
        ),
        Boot::Run
    );
    let snapshot = paths.backups().join("pre-update.sqlite");
    mam_db::snapshot(&paths.db(), &snapshot).unwrap();
    let conn = mam_db::open(&paths.db()).unwrap();
    conn.execute(
        "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
         VALUES (?1, 'future', '2026-01-01T00:00:00Z', '1')",
        [mam_db::latest_version() + 1],
    )
    .unwrap();
    snapshot
}

#[test]
fn t_upd_09_verify_update_succeeds_on_fresh_data_dir_without_touching_journal() {
    // Arrange
    let (_dir, paths) = setup();

    // Act
    let result = boot(
        &args(&[VERIFY_FLAG]),
        "0.1.1",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert
    #[cfg(not(feature = "inject-fail-verify"))]
    {
        assert_eq!(result, Boot::Exit(0));
        assert!(paths.db().is_file());
        assert!(!paths.journal().exists());
    }
    #[cfg(feature = "inject-fail-verify")]
    assert_eq!(result, Boot::Exit(1));
}

#[test]
fn t_upd_10_verify_update_fails_on_db_newer_than_app_and_leaves_it_unchanged() {
    // Arrange
    let (_dir, paths) = setup();
    make_db_newer_than_app(&paths);
    let before = std::fs::read(paths.db()).unwrap();

    // Act
    let result = boot(
        &args(&[VERIFY_FLAG]),
        "0.1.1",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert
    assert_eq!(result, Boot::Exit(1));
    assert_eq!(std::fs::read(paths.db()).unwrap(), before);
    let log = std::fs::read_to_string(paths.root().join("logs/startup.log")).unwrap();
    assert!(log.contains("verify-update failed"), "{log}");
}

#[test]
fn t_upd_11_repeated_failed_starts_launch_previous_installer_exactly_once() {
    // Arrange: 업데이트 직후 pending journal, 이전 설치기는 있다
    let (_dir, paths) = setup();
    std::fs::create_dir_all(paths.updates()).unwrap();
    std::fs::write(paths.current_installer(), b"old-installer").unwrap();
    update_guard::prepare_update(&paths, "0.1.0", "0.1.1", None, &clock()).unwrap();
    let launcher = FakeLauncher::default();
    // 한도까지 실패한 기동을 흉내: journal 의 failed_starts 를 한도 초과로
    let raw = std::fs::read_to_string(paths.journal()).unwrap();
    let raw = raw.replace(
        "\"failed_starts\": 0",
        &format!("\"failed_starts\": {}", MAX_FAILED_STARTS + 1),
    );
    std::fs::write(paths.journal(), raw).unwrap();

    // Act
    let first = boot(&args(&[]), "0.1.1", &paths, &clock(), &launcher);
    let second = boot(&args(&[]), "0.1.1", &paths, &clock(), &launcher);

    // Assert
    assert_eq!(first, Boot::Exit(0));
    assert_eq!(second, Boot::Exit(0));
    assert_eq!(*launcher.calls.borrow(), vec![paths.current_installer()]);
}

#[test]
fn t_upd_12_restored_old_version_restores_db_from_backup_then_opens_it() {
    // Arrange: DB 는 새 스키마(구 앱이 못 엶), 백업은 구 스키마. 훅이 복구 표지를 남겼다
    let (_dir, paths) = setup();
    let snapshot = make_db_newer_than_app(&paths);
    assert_eq!(applied(&paths).len() as i64, mam_db::latest_version() + 1);
    update_guard::prepare_update(&paths, "0.1.0", "0.1.1", Some(snapshot), &clock()).unwrap();
    std::fs::write(paths.rollback_flag(), b"").unwrap();

    // Act: 복구된 0.1.0 이 첫 기동
    let result = boot(
        &args(&[]),
        "0.1.0",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert
    assert_eq!(result, Boot::Run);
    assert_eq!(
        applied(&paths),
        (1..=mam_db::latest_version()).collect::<Vec<_>>()
    );
    let journal: crate::update_guard::Journal =
        serde_json::from_slice(&std::fs::read(paths.journal()).unwrap()).unwrap();
    assert_eq!(journal.state, State::RolledBack);
    assert!(!paths.rollback_flag().exists());
}

#[test]
fn t_upd_12_cancelled_update_does_not_restore_db() {
    // Arrange: 업데이트 설치기를 취소해 구 버전이 그대로 뜬 경우 — 그 사이 쌓인 DB 변경을 지우면 안 된다
    let (_dir, paths) = setup();
    let snapshot = make_db_newer_than_app(&paths);
    update_guard::prepare_update(&paths, "0.1.0", "0.1.1", Some(snapshot), &clock()).unwrap();
    let before = applied(&paths);

    // Act
    let result = boot(
        &args(&[]),
        "0.1.0",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert: 새 스키마 DB 라 구 앱은 열 수 없지만(Exit 1), DB 는 복원되지 않고 그대로다
    assert_eq!(result, Boot::Exit(1));
    assert_eq!(applied(&paths), before);
    assert!(!paths.journal().exists());
}

#[test]
fn t_upd_13_launcher_failure_exits_nonzero_and_does_not_retry_in_progress_window() {
    // Arrange
    let (_dir, paths) = setup();
    update_guard::prepare_update(&paths, "0.1.0", "0.1.1", None, &clock()).unwrap();
    let raw = std::fs::read_to_string(paths.journal())
        .unwrap()
        .replace("\"failed_starts\": 0", "\"failed_starts\": 9");
    std::fs::write(paths.journal(), raw).unwrap();
    let launcher = FakeLauncher {
        fail: true,
        ..Default::default()
    };

    // Act
    let first = boot(&args(&[]), "0.1.1", &paths, &clock(), &launcher);
    let second = boot(&args(&[]), "0.1.1", &paths, &clock(), &launcher);

    // Assert
    assert_eq!(first, Boot::Exit(1));
    assert_eq!(
        second,
        Boot::Exit(0),
        "rolling_back is not re-run until stale"
    );
    assert_eq!(launcher.calls.borrow().len(), 1);
}

#[test]
fn t_prot_24_pending_reset_is_applied_after_the_journal_and_before_the_db_opens() {
    // Arrange: 데이터가 있는 정상 DB + 초기화 직전 백업 + 표지
    let (_dir, paths) = setup();
    assert_eq!(
        boot(
            &args(&[]),
            "0.1.0",
            &paths,
            &clock(),
            &FakeLauncher::default()
        ),
        Boot::Run
    );
    mam_db::open(&paths.db())
        .unwrap()
        .execute(
            "INSERT INTO settings (key, value) VALUES ('keep_me_not', '1')",
            [],
        )
        .unwrap();
    let info = mam_db::create_backup(
        &paths.db(),
        &paths.backups(),
        mam_db::BackupKind::PreReset,
        7,
        &[],
        &clock(),
    )
    .unwrap();
    let backup = paths.backups().join(&info.file_name);
    let plan = mam_core::protect::plan_reset(
        &paths,
        &mam_core::protect::BackupOutcome::Created(backup.clone()),
        &[],
    )
    .unwrap();
    mam_core::protect::write_reset_marker(&paths, &clock(), &plan).unwrap();

    // Act: 재시작
    let result = boot(
        &args(&[]),
        "0.1.0",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert: 새 DB 로 마이그레이션까지 끝났고, 백업은 그대로, 표지는 소모됨
    assert_eq!(result, Boot::Run);
    assert!(!crate::reset::is_pending(&paths));
    assert!(backup.is_file());
    let conn = mam_db::open(&paths.db()).unwrap();
    let leftover: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'keep_me_not'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 0, "old data wiped");
    assert!(
        !applied(&paths).is_empty(),
        "schema migrated after the wipe"
    );
}

#[test]
fn t_prot_25_pending_reset_never_runs_in_verify_update_mode() {
    // Arrange: 표지가 있어도 NSIS 검증 모드는 데이터를 지우지 않는다
    let (_dir, paths) = setup();
    assert_eq!(
        boot(
            &args(&[]),
            "0.1.0",
            &paths,
            &clock(),
            &FakeLauncher::default()
        ),
        Boot::Run
    );
    let backup = paths.backups().join("mam-pre-reset-1.sqlite");
    std::fs::create_dir_all(paths.backups()).unwrap();
    std::fs::write(&backup, b"b").unwrap();
    let plan = mam_core::protect::plan_reset(
        &paths,
        &mam_core::protect::BackupOutcome::Created(backup),
        &[],
    )
    .unwrap();
    mam_core::protect::write_reset_marker(&paths, &clock(), &plan).unwrap();

    // Act
    boot(
        &args(&[VERIFY_FLAG]),
        "0.1.0",
        &paths,
        &clock(),
        &FakeLauncher::default(),
    );

    // Assert
    assert!(crate::reset::is_pending(&paths));
    assert!(paths.db().is_file());
}
