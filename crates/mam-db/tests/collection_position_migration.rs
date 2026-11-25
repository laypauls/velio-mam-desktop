//! V12 `collection_assets.position` 마이그레이션 시험 (T-cpos-db-*) — 기존 행이 있는 V11 DB 에 추가형으로 적용된다.
//!
//! 분류: Happy (기존 행 순번 채움) / Boundary (빈 컬렉션·컬렉션 간 독립) / Business rule (구버전 INSERT 는 NULL 허용·재실행 no-op) /
//! Performance 계약 (이웃 조회가 `idx_collection_assets_position` 사용)

mod common;

use common::{FixedClock, T0, TestDb, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

const STEP: i64 = 1 << 20;

fn v11_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto11: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 11)
        .collect();
    migrate_with(&mut conn, &upto11, &backup_dir, &FixedClock(T0)).unwrap();
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
         VALUES (?1, 'image', 10, 0, 0, 0)",
        [id],
    )
    .unwrap();
}

fn add_collection(conn: &Connection, id: i64) {
    conn.execute(
        "INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (?1, ?2, ?2, 0)",
        rusqlite::params![id, format!("c{id}")],
    )
    .unwrap();
}

fn link(conn: &Connection, cid: i64, aid: i64, added_at: i64) {
    conn.execute(
        "INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (?1, ?2, ?3)",
        [cid, aid, added_at],
    )
    .unwrap();
}

fn positions(conn: &Connection, cid: i64) -> Vec<(i64, Option<i64>)> {
    let mut stmt = conn
        .prepare("SELECT asset_id, position FROM collection_assets WHERE collection_id = ?1 ORDER BY asset_id")
        .unwrap();
    stmt.query_map([cid], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn t_cpos_db_01_v12_ranks_existing_rows_per_collection_by_added_at_then_asset_id() {
    // Arrange: 컬렉션 1 (added_at 역순 + 같은 added_at 동률), 컬렉션 2 (독립 순번), 컬렉션 3 (빈 컬렉션)
    let mut db = v11_db();
    for id in 1..=5 {
        add_asset(&db.conn, id);
    }
    for c in 1..=3 {
        add_collection(&db.conn, c);
    }
    link(&db.conn, 1, 1, 300);
    link(&db.conn, 1, 2, 100);
    link(&db.conn, 1, 3, 200);
    link(&db.conn, 1, 4, 200); // 3 과 같은 added_at → asset_id 순
    link(&db.conn, 2, 5, 50);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(
        positions(&db.conn, 1),
        vec![
            (1, Some(4 * STEP)),
            (2, Some(STEP)),
            (3, Some(2 * STEP)),
            (4, Some(3 * STEP))
        ]
    );
    assert_eq!(positions(&db.conn, 2), vec![(5, Some(STEP))]);
    assert!(positions(&db.conn, 3).is_empty());
}

#[test]
fn t_cpos_db_02_old_app_insert_without_position_succeeds_and_rerun_is_a_noop() {
    // Arrange
    let mut db = v11_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    add_asset(&db.conn, 1);
    add_collection(&db.conn, 1);

    // Act: 구 버전 앱의 INSERT (position 을 모른다)
    link(&db.conn, 1, 1, 10);
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();

    // Assert
    assert_eq!(positions(&db.conn, 1), vec![(1, None)]);
    assert!(again.applied.is_empty());
}

#[test]
fn t_cpos_db_03_neighbour_and_ordered_scan_queries_use_the_position_index() {
    // Arrange
    let mut db = v11_db();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let plan = |sql: &str| -> String {
        let mut stmt = db.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        stmt.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    };

    // Act + Assert: 이웃(앞쪽 최대) 조회 — 인덱스 탐색 한 번
    let lo = plan(
        "SELECT max(position) FROM collection_assets WHERE collection_id = 1 AND position < 5 \
         AND asset_id NOT IN (SELECT value FROM json_each('[1]'))",
    );
    println!("PLAN neighbour lo: {lo}");
    assert!(lo.contains("idx_collection_assets_position"), "{lo}");
    // 맨 앞/맨 뒤 이웃
    let hi = plan("SELECT min(position) FROM collection_assets WHERE collection_id = 1 AND position > 5");
    println!("PLAN neighbour hi: {hi}");
    assert!(hi.contains("idx_collection_assets_position"), "{hi}");
    let last = plan("SELECT max(position) FROM collection_assets WHERE collection_id = 1");
    assert!(last.contains("idx_collection_assets_position"), "{last}");
}
