//! V31 `ai_task_state.wait_since` 추가 (T-aiwait-01).
//! 분류: Boundary (구 행 보존 - NULL) / Business rule (추가형 컬럼, 기본 NULL).

mod common;

use common::{FixedClock, T0, insert_asset, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use tempfile::TempDir;

#[test]
fn t_aiwait_01_v31_adds_nullable_wait_since_and_keeps_old_rows_null() {
    // Arrange: V30 시점 DB 에 구 단계 행
    let dir = TempDir::new().unwrap();
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&dir.path().join("mam.sqlite")).unwrap();
    let upto30: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 30)
        .collect();
    migrate_with(&mut conn, &upto30, &backup_dir, &FixedClock(T0)).unwrap();
    let id = insert_asset(&conn);
    conn.execute(
        "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at)
         VALUES (?1, 'describe', 'queued', 0, ?2)",
        (id, T0),
    )
    .unwrap();

    // Act
    let report = migrate(&mut conn, &backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![31]);
    let since: Option<i64> = conn
        .query_row("SELECT wait_since FROM ai_task_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(since, None);
    conn.execute(
        "UPDATE ai_task_state SET wait_since = ?1 WHERE kind = 'describe'",
        [T0],
    )
    .unwrap();
}
