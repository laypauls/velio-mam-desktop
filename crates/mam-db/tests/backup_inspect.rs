//! 백업 검사·`pre_restore` 종류 (T-dbbackup-01 ~ T-dbbackup-04, 요청 20261107-123710).
//!
//! 분류: Happy (13 정상 백업 메타, 16 pre_restore 이름·분류) / Validation (14 빈 파일·없는 파일·쓰레기 바이트·이력 없는 DB) /
//! Business rule (15 quick_check 가 손상 감지). 검사는 읽기 전용이다 — 파일이 바이트 단위로 바뀌지 않는다.

mod common;

use common::{FixedClock, T0, insert_asset, migrated_db};
use mam_db::{
    BackupKind, DbError, create_backup, inspect_backup, list_backups, quick_check, snapshot,
};

#[test]
fn t_dbbackup_01_inspect_reports_schema_version_and_leaves_the_file_untouched() {
    // Arrange
    let db = migrated_db();
    insert_asset(&db.conn);
    let backup = db.dir.path().join("snap.sqlite");
    snapshot(&db.db_path, &backup).unwrap();
    let before = std::fs::read(&backup).unwrap();

    // Act
    let meta = inspect_backup(&backup).unwrap();

    // Assert
    assert_eq!(meta.schema_version, mam_db::latest_version());
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    assert!(!db.dir.path().join("snap.sqlite-wal").exists());
    assert!(!db.dir.path().join("snap.sqlite-shm").exists());
    quick_check(&backup).unwrap();
}

#[test]
fn t_dbbackup_02_inspect_rejects_missing_empty_garbage_and_history_less_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let missing = dir.path().join("missing.sqlite");
    let empty = dir.path().join("empty.sqlite");
    std::fs::write(&empty, b"").unwrap();
    let garbage = dir.path().join("garbage.sqlite");
    std::fs::write(&garbage, vec![0xAB; 8192]).unwrap();
    let no_history = dir.path().join("plain.sqlite");
    rusqlite::Connection::open(&no_history)
        .unwrap()
        .execute_batch("CREATE TABLE t (x INTEGER);")
        .unwrap();

    for path in [&missing, &empty, &garbage, &no_history] {
        let err = inspect_backup(path).unwrap_err();
        assert!(
            matches!(err, DbError::BackupInvalid { .. }),
            "{path:?}: {err:?}"
        );
    }
}

#[test]
fn t_dbbackup_03_quick_check_detects_a_damaged_backup() {
    // Arrange: 정상 백업의 가운데 페이지를 망가뜨린다
    let db = migrated_db();
    for _ in 0..50 {
        insert_asset(&db.conn);
    }
    let backup = db.dir.path().join("snap.sqlite");
    snapshot(&db.db_path, &backup).unwrap();
    let mut bytes = std::fs::read(&backup).unwrap();
    let mid = bytes.len() / 2 / 4096 * 4096;
    for b in &mut bytes[mid..mid + 4096] {
        *b = 0xFF;
    }
    std::fs::write(&backup, bytes).unwrap();

    // Act + Assert
    assert!(matches!(
        quick_check(&backup),
        Err(DbError::BackupInvalid { .. })
    ));
}

#[test]
fn t_dbbackup_04_pre_restore_backups_are_named_listed_and_kept_as_safety_backups() {
    // Arrange + Act: 안전 백업은 보관 개수와 별개로 최근 3개만 남는다
    let db = migrated_db();
    for i in 0..5 {
        create_backup(
            &db.db_path,
            &db.backup_dir,
            BackupKind::PreRestore,
            1,
            &[],
            &FixedClock(T0 + i),
        )
        .unwrap();
    }
    let listed = list_backups(&db.backup_dir).unwrap();

    // Assert
    assert_eq!(listed.len(), mam_db::SAFETY_BACKUPS_KEPT);
    assert!(listed.iter().all(|b| b.kind == BackupKind::PreRestore));
    assert!(listed[0].file_name.starts_with("mam-pre-restore-"));
    assert_eq!(serde_json::to_value(listed[0].kind).unwrap(), "pre_restore");
}
