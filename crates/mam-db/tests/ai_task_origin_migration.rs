//! V28 `ai_task_state.origin` 추가 · V29 구 대기 행 백필 (T-aioff-09, 09b).
//! 분류: Business rule (구 행 보존·백필 대상 구분) / Validation (CHECK 거부) / Boundary (STRICT·WITHOUT ROWID 테이블에 컬럼 추가).

mod common;

use common::{FixedClock, T0, TestDb, insert_asset, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use tempfile::TempDir;

/// `version` 까지만 적용된 DB.
fn db_upto(version: i64) -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let list: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| i64::from(m.version()) <= version)
        .collect();
    migrate_with(&mut conn, &list, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn insert_step(db: &TestDb, id: i64, kind: &str, state: &str) {
    db.conn
        .execute(
            "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at)
             VALUES (?1, ?2, ?3, 0, ?4)",
            (id, kind, state, T0),
        )
        .unwrap();
}

fn origin(db: &TestDb, kind: &str) -> Option<String> {
    db.conn
        .query_row(
            "SELECT origin FROM ai_task_state WHERE kind = ?1",
            [kind],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn t_aioff_09_v28_adds_nullable_origin_keeps_old_rows_null_and_rejects_unknown_values() {
    // Arrange: V27 시점 DB 에 구 단계 행, V28 까지만 적용
    let mut db = db_upto(27);
    let id = insert_asset(&db.conn);
    insert_step(&db, id, "stt", "queued");
    let upto28: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 28)
        .collect();

    // Act
    let report = migrate_with(&mut db.conn, &upto28, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert: V28 자체는 구 행을 건드리지 않고(NULL), 허용 값만 저장한다
    assert_eq!(report.applied, vec![28]);
    assert_eq!(origin(&db, "stt"), None);
    db.conn
        .execute(
            "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at, origin)
             VALUES (?1, 'embedding', 'queued', 0, ?2, 'background')",
            (id, T0),
        )
        .unwrap();
    let bad = db.conn.execute(
        "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at, origin)
         VALUES (?1, 'ocr', 'queued', 0, ?2, 'bogus')",
        (id, T0),
    );
    assert!(bad.is_err());
}

#[test]
fn t_aioff_09b_v29_marks_legacy_queued_and_running_rows_background_and_leaves_others_null() {
    // Arrange: V28 시점 DB — 상태별 구 행(NULL)과 이미 출처가 있는 행
    let mut db = db_upto(28);
    let id = insert_asset(&db.conn);
    for (kind, state) in [
        ("describe", "queued"),
        ("keywords", "running"),
        ("ocr", "done"),
        ("summary", "failed"),
        ("stt", "skipped"),
    ] {
        insert_step(&db, id, kind, state);
    }
    db.conn
        .execute(
            "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at, origin)
             VALUES (?1, 'embedding', 'queued', 0, ?2, 'manual')",
            (id, T0),
        )
        .unwrap();

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![29, 30, 31]);
    assert_eq!(origin(&db, "describe").as_deref(), Some("background"));
    assert_eq!(origin(&db, "keywords").as_deref(), Some("background"));
    assert_eq!(origin(&db, "ocr"), None);
    assert_eq!(origin(&db, "summary"), None);
    assert_eq!(origin(&db, "stt"), None);
    assert_eq!(origin(&db, "embedding").as_deref(), Some("manual"));
}
