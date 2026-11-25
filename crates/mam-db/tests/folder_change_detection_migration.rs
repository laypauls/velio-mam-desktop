//! V21 `managed_folders` 변경 감지 컬럼 마이그레이션 (T-detect-db-01..03) — 기존 행이 있는 V20 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy (기존 행 NULL, 재실행 no-op) / Validation (검증은 앱 몫 — 임의 텍스트 허용) / Business rule (구 컬럼 목록 호환)

mod common;

use common::{FixedClock, T0, TestDb, insert_volume, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use tempfile::TempDir;

fn v20_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto20: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 20)
        .collect();
    migrate_with(&mut conn, &upto20, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn insert_folder(db: &TestDb, vol: i64) {
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();
}

#[test]
fn t_detect_db_01_upgrade_from_v20_leaves_existing_folder_null() {
    // Arrange
    let mut db = v20_db();
    let vol = insert_volume(&db.conn, "v1");
    insert_folder(&db, vol);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![21, 22, 23, 24, 25, 26]);
    let row: (Option<String>, Option<String>, Option<String>) = db
        .conn
        .query_row(
            "SELECT watch_mode, watch_options, rescan_policy FROM managed_folders",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, (None, None, None));
}

#[test]
fn t_detect_db_02_rerun_is_noop_and_columns_accept_arbitrary_text() {
    // Arrange
    let mut db = v20_db();
    let vol = insert_volume(&db.conn, "v1");
    insert_folder(&db, vol);
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Act
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();
    let written = db.conn.execute(
        "UPDATE managed_folders SET watch_mode = 'future_detector', watch_options = 'not json', rescan_policy = '{garbage'",
        [],
    );

    // Assert
    assert!(again.applied.is_empty());
    assert_eq!(written.unwrap(), 1);
}

#[test]
fn t_detect_db_03_old_column_list_still_selects_and_inserts() {
    // Arrange
    let mut db = v20_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let vol = insert_volume(&db.conn, "v1");

    // Act
    insert_folder(&db, vol); // 새 컬럼을 모르는 INSERT
    let path: String = db
        .conn
        .query_row(
            "SELECT display_path FROM managed_folders WHERE enabled = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Assert
    assert_eq!(path, "a");
}
