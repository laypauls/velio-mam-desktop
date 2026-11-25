//! V25 `managed_folders.resume_enabled` 마이그레이션 (T-mig-resume-01..02) — 기존 행이 있는 V24 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy (기존 행 기본 1) / Validation (CHECK 가 0·1 외 값 거부)

mod common;

use common::{FixedClock, T0, TestDb, insert_volume, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use tempfile::TempDir;

fn v24_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto24: Vec<_> = shipped_migrations().into_iter().filter(|m| m.version() <= 24).collect();
    migrate_with(&mut conn, &upto24, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb { dir, db_path, backup_dir, conn }
}

#[test]
fn t_mig_resume_01_upgrade_from_v24_defaults_existing_folder_to_enabled() {
    // Arrange
    let mut db = v24_db();
    let vol = insert_volume(&db.conn, "v1");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at) VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![25, 26]);
    let v: i64 = db.conn.query_row("SELECT resume_enabled FROM managed_folders", [], |r| r.get(0)).unwrap();
    assert_eq!(v, 1);
}

#[test]
fn t_mig_resume_02_check_rejects_values_other_than_0_and_1() {
    // Arrange
    let mut db = v24_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let vol = insert_volume(&db.conn, "v1");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at) VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();

    // Act / Assert
    assert!(db.conn.execute("UPDATE managed_folders SET resume_enabled = 2", []).is_err());
    assert!(db.conn.execute("UPDATE managed_folders SET resume_enabled = 0", []).is_ok());
}
