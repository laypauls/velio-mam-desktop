//! V14 `managed_folders.hidden` 마이그레이션 (T-hide-db-01..02) — 기존 행이 있는 V13 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy (기존 행 hidden = 0 기본값, 재실행 no-op) / Validation (CHECK 0/1)

mod common;

use common::{FixedClock, T0, TestDb, insert_volume, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use tempfile::TempDir;

fn v13_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto13: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 13)
        .collect();
    migrate_with(&mut conn, &upto13, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

#[test]
fn t_hide_db_01_upgrade_from_v13_defaults_existing_folder_to_visible() {
    // Arrange
    let mut db = v13_db();
    let vol = insert_volume(&db.conn, "v1");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
    let hidden: i64 = db
        .conn
        .query_row("SELECT hidden FROM managed_folders", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hidden, 0);
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();
    assert!(again.applied.is_empty());
}

#[test]
fn t_hide_db_02_check_constraint_rejects_values_other_than_0_or_1() {
    // Arrange
    let mut db = v13_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let vol = insert_volume(&db.conn, "v1");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();

    // Act + Assert
    assert!(db.conn.execute("UPDATE managed_folders SET hidden = 2", []).is_err());
    assert!(db.conn.execute("UPDATE managed_folders SET hidden = NULL", []).is_err());
    db.conn.execute("UPDATE managed_folders SET hidden = 1", []).unwrap();
}
