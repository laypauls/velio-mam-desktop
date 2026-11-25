// Categories: Happy (04) / Business rule (05, 07 복구 결정·멱등) / Boundary (06 journal 없음·깨짐) /
// Time (08 주입 시계). 실제 파일시스템(임시 디렉터리) 위에서 동작을 확인한다.
use std::cell::Cell;

use tempfile::TempDir;

use super::*;
use crate::update_guard::journal::read_journal;

struct TestClock(Cell<i64>);

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.get()
    }
}

const T0: i64 = 1_700_000_000_000;

fn setup() -> (TempDir, Paths) {
    let dir = TempDir::new().unwrap();
    let paths = Paths::new(dir.path());
    (dir, paths)
}

fn clock(ms: i64) -> TestClock {
    TestClock(Cell::new(ms))
}

fn stored(paths: &Paths) -> Journal {
    match read_journal(&paths.journal(), 0).unwrap() {
        ReadOutcome::Valid(j) => j,
        _ => panic!("journal missing or broken"),
    }
}

fn prepared(paths: &Paths) -> Journal {
    prepare_update(
        paths,
        "0.1.0",
        "0.1.1",
        Some(paths.backups().join("pre-update.sqlite")),
        &clock(T0),
    )
    .unwrap()
}

#[test]
fn t_upd_04_pending_then_healthy_start_marks_healthy_and_then_does_nothing() {
    // Arrange
    let (_dir, paths) = setup();
    prepared(&paths);

    // Act: 새 버전의 첫 기동
    let decision = begin(&paths, "0.1.1", &clock(T0 + 10)).unwrap();
    mark_healthy(&paths, &clock(T0 + 20)).unwrap();

    // Assert
    assert_eq!(decision, Decision::Attempt);
    let j = stored(&paths);
    assert_eq!(j.state, State::Healthy);
    assert_eq!(j.failed_starts, 0);
    assert!(!paths.in_progress_marker().exists());
    // 이후 기동: 아무 동작 없음, journal 불변
    let before = std::fs::read(paths.journal()).unwrap();
    assert_eq!(
        begin(&paths, "0.1.1", &clock(T0 + 30)).unwrap(),
        Decision::Proceed
    );
    assert_eq!(std::fs::read(paths.journal()).unwrap(), before);
}

#[test]
fn t_upd_04_healthy_promotes_pending_installer_to_current() {
    // Arrange: NSIS 훅이 새 설치기를 pending 으로 복사해 둔 상태, 구 설치기는 current
    let (_dir, paths) = setup();
    prepared(&paths);
    std::fs::write(paths.current_installer(), b"old").unwrap();
    std::fs::write(paths.pending_installer(), b"new").unwrap();

    // Act
    begin(&paths, "0.1.1", &clock(T0)).unwrap();
    mark_healthy(&paths, &clock(T0)).unwrap();

    // Assert
    assert_eq!(std::fs::read(paths.current_installer()).unwrap(), b"new");
    assert!(!paths.pending_installer().exists());
}

#[test]
fn t_upd_05_failed_starts_over_limit_decide_rollback_to_previous_installer_and_backup() {
    // Arrange: 기동할 때마다 healthy 에 도달하지 못함
    let (_dir, paths) = setup();
    let j = prepared(&paths);

    // Act + Assert: 한도(2) 이내의 3번 시도는 모두 Attempt, 그 다음이 Rollback
    for attempt in 1..=3 {
        assert_eq!(
            begin(&paths, "0.1.1", &clock(T0 + attempt)).unwrap(),
            Decision::Attempt,
            "attempt {attempt}"
        );
        assert_eq!(stored(&paths).failed_starts, attempt as u32);
    }
    let decision = begin(&paths, "0.1.1", &clock(T0 + 4)).unwrap();
    assert_eq!(
        decision,
        Decision::Rollback {
            installer: j.previous_installer.clone(),
            backup: j.db_backup.clone(),
        }
    );
    assert_eq!(stored(&paths).state, State::RollingBack);
    // 회귀(시나리오 S3): 진행 표지가 남으면 이전 설치기의 NSIS 훅이 구 exe 를 새 스키마 DB 로 검증하다 실패한다
    assert!(!paths.in_progress_marker().exists());
}

#[test]
fn t_upd_05_boundary_exactly_at_limit_still_attempts() {
    // Arrange
    let mut j = prepared(&setup().1);
    j.failed_starts = MAX_FAILED_STARTS;

    // Act + Assert
    assert_eq!(decide(&j, "0.1.1", false, T0), Decision::Attempt);
    j.failed_starts = MAX_FAILED_STARTS + 1;
    assert!(matches!(
        decide(&j, "0.1.1", false, T0),
        Decision::Rollback { .. }
    ));
}

#[test]
fn t_upd_05_restored_old_version_completes_rollback_only_when_rollback_was_requested() {
    // Arrange
    let (_dir, paths) = setup();
    let j = prepared(&paths);

    // Act + Assert: 훅 표지 없이 구 버전이 뜨면 업데이트가 적용되지 않은 것 -> DB 를 건드리지 않고 정리
    assert_eq!(
        decide(&j, "0.1.0", false, T0),
        Decision::Cancelled,
        "cancelled installer must not restore the DB"
    );
    // 훅이 이전 설치기를 실행한 경우 -> DB 복원
    assert_eq!(
        decide(&j, "0.1.0", true, T0),
        Decision::CompleteRollback {
            backup: j.db_backup.clone()
        }
    );
    // 앱이 직접 시작한 복구(rolling_back) 뒤의 구 버전
    let mut rolling = j.clone();
    rolling.state = State::RollingBack;
    assert!(matches!(
        decide(&rolling, "0.1.0", false, T0),
        Decision::CompleteRollback { .. }
    ));
}

#[test]
fn t_upd_05_cancelled_update_removes_journal_and_marker() {
    // Arrange
    let (_dir, paths) = setup();
    prepared(&paths);

    // Act
    let decision = begin(&paths, "0.1.0", &clock(T0)).unwrap();

    // Assert
    assert_eq!(decision, Decision::Cancelled);
    assert!(!paths.journal().exists());
    assert!(!paths.in_progress_marker().exists());
}

#[test]
fn t_upd_05_rolled_back_journal_is_final() {
    // Arrange
    let (_dir, paths) = setup();
    prepared(&paths);
    std::fs::write(paths.rollback_flag(), b"").unwrap();
    let d = begin(&paths, "0.1.0", &clock(T0 + 1)).unwrap();
    assert!(matches!(d, Decision::CompleteRollback { .. }));

    // Act
    mark_rolled_back(&paths, &clock(T0 + 2)).unwrap();

    // Assert: 표지 제거, 이후 어떤 버전이 떠도 복구를 반복하지 않는다
    assert_eq!(stored(&paths).state, State::RolledBack);
    assert!(!paths.rollback_flag().exists());
    assert_eq!(
        begin(&paths, "0.1.0", &clock(T0 + 3)).unwrap(),
        Decision::Proceed
    );
    assert_eq!(
        begin(&paths, "0.1.1", &clock(T0 + 4)).unwrap(),
        Decision::Proceed
    );
}

#[test]
fn t_upd_06_missing_journal_means_normal_start() {
    // Arrange
    let (_dir, paths) = setup();

    // Act + Assert
    assert_eq!(
        begin(&paths, "0.1.1", &clock(T0)).unwrap(),
        Decision::Proceed
    );
    assert!(!paths.journal().exists());
}

#[test]
fn t_upd_06_broken_json_means_normal_start_and_file_is_preserved() {
    // Arrange
    let (_dir, paths) = setup();
    std::fs::write(paths.journal(), b"{ not json").unwrap();

    // Act
    let decision = begin(&paths, "0.1.1", &clock(T0)).unwrap();

    // Assert: 복구하지 않고, 깨진 원본은 지우지 않고 옮겨 둔다
    assert_eq!(decision, Decision::Proceed);
    assert!(!paths.journal().exists());
    let preserved = paths
        .root()
        .join(format!("update-journal.json.broken-{T0}"));
    assert_eq!(std::fs::read(preserved).unwrap(), b"{ not json");
}

#[test]
fn t_upd_06_journal_with_unknown_state_is_treated_as_broken() {
    // Arrange: 미래 버전이 쓴 알 수 없는 상태값
    let (_dir, paths) = setup();
    let mut value = serde_json::to_value(prepared(&paths)).unwrap();
    value["state"] = "exploded".into();
    std::fs::write(paths.journal(), serde_json::to_vec(&value).unwrap()).unwrap();

    // Act + Assert
    assert_eq!(
        begin(&paths, "0.1.1", &clock(T0)).unwrap(),
        Decision::Proceed
    );
}

#[test]
fn t_upd_07_rollback_decision_runs_only_once() {
    // Arrange: 한도를 넘긴 pending
    let (_dir, paths) = setup();
    prepared(&paths);
    let mut j = stored(&paths);
    j.failed_starts = MAX_FAILED_STARTS + 1;
    journal::write_journal(&paths.journal(), &j).unwrap();

    // Act
    let first = begin(&paths, "0.1.1", &clock(T0 + 1)).unwrap();
    let second = begin(&paths, "0.1.1", &clock(T0 + 2)).unwrap();

    // Assert
    assert!(matches!(first, Decision::Rollback { .. }));
    assert_eq!(second, Decision::RollbackInProgress);
}

#[test]
fn t_upd_07_stale_rolling_back_is_retried_after_timeout() {
    // Arrange: 복구 설치기가 실행되지 못한 채 방치된 경우
    let (_dir, paths) = setup();
    prepared(&paths);
    let mut j = stored(&paths);
    j.state = State::RollingBack;
    j.updated_at_ms = T0;
    journal::write_journal(&paths.journal(), &j).unwrap();

    // Act + Assert
    assert_eq!(
        decide(&j, "0.1.1", false, T0 + ROLLBACK_STALE_MS - 1),
        Decision::RollbackInProgress
    );
    assert!(matches!(
        decide(&j, "0.1.1", false, T0 + ROLLBACK_STALE_MS),
        Decision::Rollback { .. }
    ));
}

#[test]
fn t_upd_08_journal_timestamps_come_from_the_injected_clock() {
    // Arrange + Act
    let (_dir, paths) = setup();
    prepared(&paths);
    begin(&paths, "0.1.1", &clock(T0 + 500)).unwrap();

    // Assert
    let j = stored(&paths);
    assert_eq!(j.created_at_ms, T0);
    assert_eq!(j.updated_at_ms, T0 + 500);
    assert_eq!(j.from_version, "0.1.0");
    assert_eq!(j.to_version, "0.1.1");
    assert_eq!(j.previous_installer, paths.current_installer());
}

#[test]
fn t_upd_08_abort_update_clears_journal_and_marker() {
    // Arrange
    let (_dir, paths) = setup();
    prepared(&paths);
    assert!(paths.in_progress_marker().exists());

    // Act
    abort_update(&paths).unwrap();

    // Assert
    assert!(!paths.journal().exists());
    assert!(!paths.in_progress_marker().exists());
    abort_update(&paths).unwrap(); // 멱등
}
