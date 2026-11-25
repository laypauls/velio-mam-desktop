//! V20 저장한 검색 테이블 삭제 시험 (T-mig-*) — `saved_searches` 만 사라지고 `recent_searches` 는 남는다.
//!
//! 분류: Happy (01 최신 스키마) / Boundary (02 행이 있는 V19 DB 에서 적용) / Concurrency·멱등 (03 재실행 no-op)

mod common;

use common::{FixedClock, T0, TestDb, applied_versions, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v19_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto19: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 19)
        .collect();
    migrate_with(&mut conn, &upto19, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn t_mig_01_latest_schema_has_no_saved_searches_but_keeps_recent_searches() {
    // Arrange
    let mut db = v19_db();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![20, 21, 22, 23, 24, 25, 26]);
    assert!(!table_exists(&db.conn, "saved_searches"));
    assert!(table_exists(&db.conn, "recent_searches"));
}

#[test]
fn t_mig_02_v20_applies_to_a_v19_db_with_saved_rows_and_keeps_recent_rows() {
    // Arrange: 저장한 검색·최근 검색 행이 있는 V19 DB
    let mut db = v19_db();
    db.conn
        .execute(
            "INSERT INTO saved_searches (name, name_key, query, filters_json, sort_json, created_at)
             VALUES ('Beach', 'beach', 'sea', '{}', '{}', 1)",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "INSERT INTO recent_searches (query, filters_json, used_at) VALUES ('sea', '{}', 1)",
            [],
        )
        .unwrap();
    assert_eq!(applied_versions(&db.conn).last(), Some(&19));

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![20, 21, 22, 23, 24, 25, 26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert!(!table_exists(&db.conn, "saved_searches"));
    let recent: i64 = db
        .conn
        .query_row("SELECT count(*) FROM recent_searches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(recent, 1);
}

#[test]
fn t_mig_03_rerunning_after_v20_is_a_noop() {
    // Arrange
    let mut db = v19_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Act
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();

    // Assert
    assert!(again.applied.is_empty());
    assert_eq!(applied_versions(&db.conn).last(), Some(&26));
}
