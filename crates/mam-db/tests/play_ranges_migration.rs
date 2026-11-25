//! V13 `asset_play_ranges` 마이그레이션 (T-ranges-14) — 기존 행이 있는 V12 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy (V12 에서 업그레이드, 기존 행 보존, 재실행 no-op) / Validation (CHECK·FK·PK 제약) / Business rule (CASCADE)

mod common;

use common::{FixedClock, T0, TestDb, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v12_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto12: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 12)
        .collect();
    migrate_with(&mut conn, &upto12, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn add_asset(conn: &Connection, id: i64) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 'video', 10, 0, 0, 0)",
        [id],
    )
    .unwrap();
}

fn put(conn: &Connection, id: i64, s: i64, e: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO asset_play_ranges (asset_id, start_ms, end_ms) VALUES (?1, ?2, ?3)",
        [id, s, e],
    )
}

#[test]
fn t_ranges_14_upgrade_from_v12_adds_table_and_keeps_existing_rows() {
    // Arrange
    let mut db = v12_db();
    add_asset(&db.conn, 1);
    let has: bool = db
        .conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE name = 'asset_play_ranges'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!has);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(
        report.applied,
        vec![13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27]
    );
    assert!(report.backup.is_some_and(|b| b.is_file()));
    let assets: i64 = db
        .conn
        .query_row("SELECT count(*) FROM media_assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(assets, 1);
    put(&db.conn, 1, 0, 1000).unwrap();
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();
    assert!(again.applied.is_empty());
}

#[test]
fn t_ranges_14_constraints_reject_bad_rows_and_cascade_on_asset_delete() {
    // Arrange
    let mut db = v12_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    add_asset(&db.conn, 1);
    add_asset(&db.conn, 2);

    // Act + Assert: 실패 경로
    assert!(put(&db.conn, 1, -1, 10).is_err(), "start >= 0");
    assert!(put(&db.conn, 1, 10, 10).is_err(), "end > start");
    assert!(put(&db.conn, 1, 10, 5).is_err(), "end > start");
    assert!(put(&db.conn, 99, 0, 10).is_err(), "FK: 없는 자산");
    put(&db.conn, 1, 0, 10).unwrap();
    assert!(put(&db.conn, 1, 0, 20).is_err(), "PK (asset_id, start_ms)");
    put(&db.conn, 2, 0, 10).unwrap();

    // CASCADE: 자산 삭제 시 그 자산의 구간만 사라진다
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 1", [])
        .unwrap();
    let left: Vec<i64> = db
        .conn
        .prepare("SELECT asset_id FROM asset_play_ranges")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(left, vec![2]);
}
