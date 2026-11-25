//! 백업 복원 통합 테스트 (T-upd-01 ~ T-upd-03, P1-7).
//!
//! 분류: Happy (01) / External (02 없는·손상된 백업) / Business rule (03 롤백 후 구 러너가 연다).
//! 실제 SQLite 임시 파일을 쓰고, 실패 경로는 "대상 DB 파일이 바이트 단위로 무변경"을 단언한다.

mod common;

use common::{
    FixedClock, T0, applied_versions, count, dir_entries, future_migration, insert_asset,
    migrated_db, shipped_migrations, v1, v1_db, v2,
};
use mam_db::{DbError, migrate, migrate_with, open, restore_backup, snapshot};

fn file_bytes(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn t_upd_01_restore_returns_db_to_backup_time_state() {
    // Arrange: 자산 1건 -> 스냅샷 -> 자산 추가(WAL 에 남은 상태)
    let db = migrated_db();
    insert_asset(&db.conn);
    let backup = db.dir.path().join("snap.sqlite");
    snapshot(&db.db_path, &backup).unwrap();
    insert_asset(&db.conn);
    insert_asset(&db.conn);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 3);
    let db_path = db.db_path.clone();
    let dir = db.dir;
    drop(db.conn); // Windows 는 열린 파일을 교체할 수 없다

    // Act
    restore_backup(&backup, &db_path).unwrap();

    // Assert
    let conn = open(&db_path).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(
        applied_versions(&conn),
        (1..=mam_db::latest_version()).collect::<Vec<_>>()
    );
    let names = dir_entries(dir.path());
    assert!(
        !names.iter().any(|n| n.ends_with(".restore-tmp")),
        "temp file left: {names:?}"
    );
    assert!(backup.is_file(), "backup is kept");
}

#[test]
fn t_upd_01_restore_into_missing_target_creates_it() {
    // Arrange
    let db = migrated_db();
    insert_asset(&db.conn);
    let backup = db.dir.path().join("snap.sqlite");
    snapshot(&db.db_path, &backup).unwrap();
    let target = db.dir.path().join("new").join("mam.sqlite");

    // Act
    restore_backup(&backup, &target).unwrap();

    // Assert
    let conn = open(&target).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM media_assets"), 1);
}

#[test]
fn t_upd_01_snapshot_refuses_to_overwrite_existing_file() {
    // Arrange
    let db = migrated_db();
    let backup = db.dir.path().join("snap.sqlite");
    std::fs::write(&backup, b"keep me").unwrap();

    // Act
    let err = snapshot(&db.db_path, &backup).unwrap_err();

    // Assert
    assert!(matches!(err, DbError::BackupExists(_)), "{err:?}");
    assert_eq!(file_bytes(&backup), b"keep me");
}

#[test]
fn t_upd_02_missing_backup_fails_and_target_is_untouched() {
    // Arrange
    let db = migrated_db();
    insert_asset(&db.conn);
    let missing = db.dir.path().join("nope.sqlite");
    let db_path = db.db_path.clone();
    drop(db.conn);
    let before = file_bytes(&db_path);

    // Act
    let err = restore_backup(&missing, &db_path).unwrap_err();

    // Assert
    assert!(matches!(err, DbError::BackupInvalid { .. }), "{err:?}");
    assert_eq!(file_bytes(&db_path), before);
}

#[test]
fn t_upd_02_garbage_and_truncated_backups_fail_and_target_is_untouched() {
    // Arrange: 유효한 스냅샷을 잘라낸 것 / SQLite 가 아닌 파일 / 빈 파일
    let db = migrated_db();
    insert_asset(&db.conn);
    let good = db.dir.path().join("good.sqlite");
    snapshot(&db.db_path, &good).unwrap();
    let bytes = file_bytes(&good);
    let truncated = db.dir.path().join("truncated.sqlite");
    std::fs::write(&truncated, &bytes[..bytes.len() / 2]).unwrap();
    let garbage = db.dir.path().join("garbage.sqlite");
    std::fs::write(&garbage, vec![0xAB; 8192]).unwrap();
    let empty = db.dir.path().join("empty.sqlite");
    std::fs::write(&empty, b"").unwrap();
    let db_path = db.db_path.clone();
    drop(db.conn);
    let before = file_bytes(&db_path);

    for bad in [&truncated, &garbage, &empty] {
        // Act
        let res = restore_backup(bad, &db_path);

        // Assert (빈 파일은 SQLite 가 빈 DB 로 열지만 스키마가 없어 거부돼야 한다)
        let err = res.expect_err(&format!("{bad:?} must be rejected"));
        assert!(matches!(err, DbError::BackupInvalid { .. }), "{err:?}");
        assert_eq!(file_bytes(&db_path), before, "target changed by {bad:?}");
    }
    let names = dir_entries(db.dir.path());
    assert!(
        !names.iter().any(|n| n.ends_with(".restore-tmp")),
        "{names:?}"
    );
}

#[test]
fn t_upd_03_v1_backup_restores_a_db_migrated_to_v2_and_v1_runner_opens_it() {
    // Arrange: 최신 스키마 DB(자산 1건) -> 미래 마이그레이션(적용 전 백업 생성) -> 구 앱(최신 기준)은 SchemaTooNew
    let mut db = migrated_db();
    insert_asset(&db.conn);
    let mut all = shipped_migrations();
    all.push(future_migration());
    let report = migrate_with(&mut db.conn, &all, &db.backup_dir, &FixedClock(T0)).unwrap();
    let backup = report.backup.expect("pre-migrate backup");
    assert!(matches!(
        migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)),
        Err(DbError::SchemaTooNew { .. })
    ));
    let db_path = db.db_path.clone();
    let backup_dir = db.backup_dir.clone();
    drop(db.conn);

    // Act
    restore_backup(&backup, &db_path).unwrap();

    // Assert
    let mut conn = open(&db_path).unwrap();
    let report = migrate(&mut conn, &backup_dir, &FixedClock(T0 + 2)).unwrap();
    assert!(report.applied.is_empty() && report.backup.is_none());
    assert_eq!(
        applied_versions(&conn),
        (1..=mam_db::latest_version()).collect::<Vec<_>>()
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM media_assets"), 1);
    let future: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'future_table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(future, 0, "v2 schema is gone");
}

#[test]
fn t_upd_03_stale_wal_of_the_new_db_is_not_replayed_onto_the_restored_file() {
    // Arrange: 새 스키마 DB 에 WAL 로 남은 쓰기가 있는 채(연결을 닫지 않고 -wal 이 존재) 연결만 놓는다
    let mut db = v1_db();
    let report =
        migrate_with(&mut db.conn, &[v1(), v2()], &db.backup_dir, &FixedClock(T0)).unwrap();
    let backup = report.backup.unwrap();
    insert_asset(&db.conn);
    let db_path = db.db_path.clone();
    drop(db.conn);

    // Act
    restore_backup(&backup, &db_path).unwrap();

    // Assert: 복원본에는 백업 이후의 자산이 없고, 잔여 WAL 파일도 없다
    let conn = open(&db_path).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM media_assets"), 0);
    drop(conn);
    let names = dir_entries(db.dir.path());
    assert!(
        !names.iter().any(|n| n.ends_with(".restore-tmp")),
        "{names:?}"
    );
}
