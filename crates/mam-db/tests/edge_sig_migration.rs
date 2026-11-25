//! V11 `media_assets.edge_sig` 마이그레이션 시험 (T-edge-db-*) — 기존 행이 있는 V10 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy / Boundary (기존 행 NULL) / Validation (CHECK 길이 32) / Business rule (구버전 INSERT 호환·재실행 no-op) /
//! Performance 계약 (같은 크기·같은 edge_sig 조회가 부분 인덱스를 쓴다)

mod common;

use common::{FixedClock, T0, TestDb, migrated_db, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v10_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto10: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 10)
        .collect();
    migrate_with(&mut conn, &upto10, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn add_asset(conn: &Connection, id: i64, size: i64) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 'image', ?2, 0, 0, 0)",
        [id, size],
    )
    .unwrap();
}

fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn t_edge_db_01_v11_applies_to_a_v10_db_keeping_existing_rows_without_edge_sig() {
    // Arrange: edge_sig 를 모르는 앱이 만든 자산
    let mut db = v10_db();
    add_asset(&db.conn, 1, 10);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    let sig: Option<Vec<u8>> = db
        .conn
        .query_row("SELECT edge_sig FROM media_assets WHERE asset_id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sig, None);
}

#[test]
fn t_edge_db_02_check_constraint_requires_exactly_32_bytes() {
    // Arrange
    let db = migrated_db();
    add_asset(&db.conn, 1, 10);

    // Act / Assert
    for bad in [0usize, 31, 33] {
        let r = db.conn.execute(
            "UPDATE media_assets SET edge_sig = zeroblob(?1) WHERE asset_id = 1",
            [bad as i64],
        );
        assert!(r.is_err(), "length {bad} must violate the CHECK");
    }
    db.conn
        .execute("UPDATE media_assets SET edge_sig = zeroblob(32) WHERE asset_id = 1", [])
        .unwrap();
    db.conn
        .execute("UPDATE media_assets SET edge_sig = NULL WHERE asset_id = 1", [])
        .unwrap();
}

#[test]
fn t_edge_db_03_old_style_insert_works_and_rerun_is_a_noop() {
    // Arrange
    let mut db = v10_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Act: 구 버전 앱의 INSERT (edge_sig 컬럼을 모른다)
    add_asset(&db.conn, 7, 5);
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();

    // Assert
    assert!(again.applied.is_empty());
    assert_eq!(common::count(&db.conn, "SELECT count(*) FROM media_assets WHERE edge_sig IS NULL"), 1);
}

#[test]
fn t_edge_db_04_same_size_and_edge_lookup_uses_the_partial_index() {
    // Arrange
    let db = migrated_db();

    // Act / Assert: 중복 검사 충돌 조회 (processing/edge.rs COLLISION_SQL 과 같은 모양 - 리터럴 status 로 부분 인덱스를 쓴다)
    for sql in [
        "SELECT EXISTS (SELECT 1 FROM media_assets
           WHERE status = 'active' AND file_size = 100 AND edge_sig = x'00' AND asset_id <> 1)",
        // 백필: 같은 크기·같은 sig 상대가 있는 pending 자산 (상관 부질의)
        "SELECT a.asset_id FROM media_assets a
         WHERE a.status = 'active' AND a.hash_state = 'pending' AND EXISTS (
           SELECT 1 FROM media_assets b
           WHERE b.status = 'active' AND b.file_size = a.file_size AND b.edge_sig = a.edge_sig
             AND b.asset_id <> a.asset_id)",
    ] {
        let p = plan(&db.conn, sql);
        assert!(p.contains("idx_media_assets_edge"), "expected idx_media_assets_edge: {p}");
    }
    // 같은 크기·같은 sig 상대에게 hash 작업을 등록하는 SELECT 도 같다
    let p = plan(
        &db.conn,
        "SELECT asset_id FROM media_assets
         WHERE status = 'active' AND hash_state = 'pending'
           AND file_size = 100 AND edge_sig = x'00' AND asset_id <> 1",
    );
    assert!(p.contains("idx_media_assets_edge"), "{p}");
}
