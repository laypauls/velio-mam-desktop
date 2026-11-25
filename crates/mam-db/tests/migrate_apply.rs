//! 마이그레이션 적용·백업 통합 테스트 (T-db-01, T-db-02, T-db-03, T-db-05).
//!
//! 분류: Happy (01, 05) / Boundary (05 새 DB) / Concurrency-멱등 (02) / External failure (03).
//! 실제 SQLite 임시 파일을 쓰고, 백업 파일명 시각은 주입한 고정 시계를 쓴다.

mod common;

use common::{
    FixedClock, T0, applied_versions, dir_entries, empty_db, insert_volume, migrated_db,
    schema_dump, user_tables, v1, v1_db, v2,
};
use mam_db::{DbError, Migration, latest_version, migrate, migrate_with, open};
use rusqlite::Connection;

#[test]
fn t_db_01_fresh_db_gets_full_schema_and_pragmas() {
    // Arrange
    let mut db = empty_db();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0)).unwrap();

    // Assert
    assert_eq!(report.applied, (1..=latest_version()).collect::<Vec<_>>());
    assert_eq!(report.backup, None);
    let tables = user_tables(&db.conn);
    for expected in [
        "asset_relations",
        "asset_tags",
        "file_locations",
        "index_progress",
        "media_assets",
        "media_metadata",
        "search_outbox",
        "settings",
        "tags",
        "volumes",
    ] {
        assert!(tables.contains(&expected.to_string()), "missing {expected}");
    }
    let fk: i64 = db
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    let mode: String = db
        .conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    let busy: i64 = db
        .conn
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    assert_eq!(mode, "wal");
    assert_eq!(busy, mam_db::BUSY_TIMEOUT.as_millis() as i64);
}

#[test]
fn t_db_01_every_new_connection_enables_foreign_keys() {
    // Arrange: 마이그레이션된 DB 를 다시 연다
    let db = migrated_db();

    // Act
    let reopened = open(&db.db_path).unwrap();

    // Assert
    let fk: i64 = reopened
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
}

#[test]
fn t_db_02_rerun_is_noop_and_creates_no_backup() {
    // Arrange
    let mut db = migrated_db();
    let before = schema_dump(&db.conn);
    let versions = applied_versions(&db.conn);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert!(report.applied.is_empty());
    assert_eq!(report.backup, None);
    assert_eq!(schema_dump(&db.conn), before);
    assert_eq!(applied_versions(&db.conn), versions);
    assert!(dir_entries(&db.backup_dir).is_empty());
}

#[test]
fn t_db_03_failing_migration_rolls_back_everything_on_fresh_db() {
    // Arrange: V1 은 정상, V2 는 테이블을 만든 뒤 실패한다 (같은 실행에서 적용)
    let mut db = empty_db();
    let bad = Migration::unapplied(
        "V2__bad",
        "CREATE TABLE half_done (a INTEGER) STRICT; INSERT INTO no_such_table VALUES (1);",
    )
    .unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), bad], &db.backup_dir, &FixedClock(T0));

    // Assert: V1 포함 어떤 스키마도 남지 않고 적용 이력도 비어 있다
    assert!(
        matches!(err, Err(DbError::ApplyFailed { backup: None, .. })),
        "{err:?}"
    );
    let tables: Vec<String> = user_tables(&db.conn)
        .into_iter()
        .filter(|t| t != "refinery_schema_history")
        .collect();
    assert!(tables.is_empty(), "leftover tables: {tables:?}");
    assert!(applied_versions(&db.conn).is_empty());
}

#[test]
fn t_db_03_failing_migration_leaves_existing_db_unchanged() {
    // Arrange: V1 적용된 DB 에 데이터가 있고, 대기 V2 가 중간에 실패한다
    let mut db = v1_db();
    insert_volume(&db.conn, "vol-a");
    let before = schema_dump(&db.conn);
    let bad = Migration::unapplied(
        "V2__bad",
        "CREATE TABLE half_done (a INTEGER) STRICT; INSERT INTO no_such_table VALUES (1);",
    )
    .unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), bad], &db.backup_dir, &FixedClock(T0));

    // Assert
    let Err(DbError::ApplyFailed {
        backup: Some(backup),
        ..
    }) = err
    else {
        panic!("expected ApplyFailed carrying the backup path, got {err:?}");
    };
    assert!(backup.is_file(), "backup path must point to a real file");
    assert_eq!(schema_dump(&db.conn), before);
    assert_eq!(applied_versions(&db.conn), vec![1]);
    let volumes: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM volumes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(volumes, 1);
}

#[test]
fn t_db_05_existing_db_with_pending_migration_is_backed_up_first() {
    // Arrange: V1 적용 + 데이터 1행 (WAL 에만 있을 수 있는 쓰기 포함)
    let mut db = v1_db();
    insert_volume(&db.conn, "vol-a");

    // Act
    let report = migrate_with(
        &mut db.conn,
        &[v1(), v2()],
        &db.backup_dir,
        &FixedClock(T0 + 42),
    )
    .unwrap();

    // Assert: 파일명은 주입 시계 기준, 백업은 "적용 전" 스냅샷이며 무결하다
    assert_eq!(report.applied, vec![2]);
    let backup = report.backup.expect("backup must be created");
    assert_eq!(
        backup.file_name().unwrap().to_string_lossy(),
        format!("mam-pre-migrate-v1-{}.sqlite", T0 + 42)
    );
    let snap = Connection::open(&backup).unwrap();
    let check: String = snap
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    assert!(user_tables(&snap).contains(&"volumes".to_string()));
    assert!(!user_tables(&snap).contains(&"future_table".to_string()));
    let volumes: i64 = snap
        .query_row("SELECT COUNT(*) FROM volumes", [], |r| r.get(0))
        .unwrap();
    assert_eq!(volumes, 1);
    assert!(user_tables(&db.conn).contains(&"future_table".to_string()));
}

#[test]
fn t_db_05_fresh_db_is_not_backed_up() {
    // Arrange
    let mut db = empty_db();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0)).unwrap();

    // Assert
    assert_eq!(report.backup, None);
    assert!(dir_entries(&db.backup_dir).is_empty());
}

#[test]
fn t_db_05_backup_name_collision_aborts_before_applying() {
    // Arrange: 같은 시각의 백업 파일이 이미 있다
    let mut db = v1_db();
    std::fs::create_dir_all(&db.backup_dir).unwrap();
    let occupied = db
        .backup_dir
        .join(format!("mam-pre-migrate-v1-{T0}.sqlite"));
    std::fs::write(&occupied, b"occupied").unwrap();

    // Act
    let err = migrate_with(&mut db.conn, &[v1(), v2()], &db.backup_dir, &FixedClock(T0));

    // Assert: 기존 파일을 덮어쓰지 않고, 마이그레이션도 적용하지 않는다
    assert!(matches!(err, Err(DbError::BackupExists(_))), "{err:?}");
    assert_eq!(applied_versions(&db.conn), vec![1]);
    assert_eq!(std::fs::read(&occupied).unwrap(), b"occupied");
}
