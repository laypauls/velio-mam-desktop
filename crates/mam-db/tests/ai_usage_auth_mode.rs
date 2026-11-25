//! V8 `ai_usage.auth_mode` 마이그레이션 시험 (T-db-aiauth-*) — 기존 사용량 행이 있는 V7 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy / Boundary(기존 행 NULL) / Business rule(추가형·구버전 INSERT 호환)

mod common;

use common::{FixedClock, T0, TestDb, applied_versions, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

/// V7 까지만 적용된 DB (V8 이전 앱이 쓰던 상태).
fn v7_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto7: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 7)
        .collect();
    migrate_with(&mut conn, &upto7, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn old_style_insert(conn: &Connection) {
    // V8 이전 앱의 INSERT — auth_mode 컬럼을 모른다
    conn.execute(
        "INSERT INTO ai_usage (provider, model, kind, ok, at) VALUES ('openai', 'gpt', 'describe', 1, ?1)",
        [T0],
    )
    .unwrap();
}

#[test]
fn t_db_aiauth_01_v8_applies_to_a_v7_db_keeping_existing_rows_with_null_auth_mode() {
    // Arrange
    let mut db = v7_db();
    old_style_insert(&db.conn);
    assert_eq!(applied_versions(&db.conn).last(), Some(&7));

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert: V8 이 첫 대기 마이그레이션으로 적용되고 적용 전 백업이 만들어진다
    assert_eq!(
        report.applied.first(),
        Some(&8),
        "V8 부터 대기 중인 것만 적용"
    );
    assert!(report.backup.is_some_and(|b| b.is_file()));
    let (n, nulls): (i64, i64) = db
        .conn
        .query_row(
            "SELECT COUNT(*), SUM(auth_mode IS NULL) FROM ai_usage",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((n, nulls), (1, 1), "기존 행은 NULL(= apiKey)");
}

#[test]
fn t_db_aiauth_02_new_column_is_optional_so_old_and_new_inserts_both_work() {
    let mut db = v7_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    old_style_insert(&db.conn);
    db.conn
        .execute(
            "INSERT INTO ai_usage (provider, model, kind, ok, at, auth_mode) VALUES ('claude', 'haiku', 'check', 1, ?1, 'subscription')",
            [T0],
        )
        .unwrap();

    let modes: Vec<Option<String>> = db
        .conn
        .prepare("SELECT auth_mode FROM ai_usage ORDER BY usage_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(modes, vec![None, Some("subscription".to_string())]);
}

#[test]
fn t_db_aiauth_03_fresh_db_gets_the_column_and_rerun_is_a_noop() {
    let mut db = common::empty_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0)).unwrap();

    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    assert!(again.applied.is_empty());
    let has: bool = db
        .conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM pragma_table_info('ai_usage') WHERE name = 'auth_mode')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has);
}
