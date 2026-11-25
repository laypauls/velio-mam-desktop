//! 핵심 쿼리 인덱스 사용 계약 (T-db-12).
//!
//! 분류: Performance 계약 (EXPLAIN QUERY PLAN 으로 인덱스 사용만 확인 — 100만 건 실측은 P1-2).
//! 부분 인덱스는 SQLite 가 WHERE 절이 인덱스 조건을 함의함을 증명할 수 있어야 쓰이므로
//! 쿼리 모양(리터럴 상태값 등)까지 계약으로 고정한다. 앱 쿼리를 바꾸면 이 테스트도 같이 본다.

mod common;

use common::migrated_db;
use rusqlite::Connection;

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn assert_uses_index(conn: &Connection, sql: &str, index: &str) {
    let details = plan(conn, sql);
    let joined = details.join(" | ");
    assert!(
        details
            .iter()
            .any(|d| d.starts_with("SEARCH") && d.contains(index)),
        "expected SEARCH using {index}, got: {joined}"
    );
    assert!(
        !details.iter().any(|d| d.starts_with("SCAN")),
        "full scan in plan: {joined}"
    );
}

#[test]
fn t_db_12_path_lookup_uses_active_path_index() {
    // Arrange
    let db = migrated_db();

    // Act / Assert
    assert_uses_index(
        &db.conn,
        "SELECT asset_id FROM file_locations
         WHERE volume_id = 1 AND path_key = 'a/b.jpg' AND detached_at IS NULL",
        "uq_file_locations_active_path",
    );
}

#[test]
fn t_db_12_hash_lookup_uses_algo_hash_index() {
    let db = migrated_db();

    assert_uses_index(
        &db.conn,
        "SELECT asset_id FROM media_assets
         WHERE hash_algo = 'blake3' AND content_hash = x'00112233445566778899aabbccddeeff'",
        "idx_media_assets_hash",
    );
}

#[test]
fn t_db_12_locations_by_asset_use_asset_index() {
    let db = migrated_db();

    assert_uses_index(
        &db.conn,
        "SELECT location_id, volume_id, path_key FROM file_locations WHERE asset_id = 7",
        "idx_file_locations_asset",
    );
}

#[test]
fn t_db_12_pending_hash_scan_uses_partial_index() {
    let db = migrated_db();

    let sql = "SELECT asset_id FROM media_assets WHERE hash_state = 'pending' ORDER BY asset_id LIMIT 100";
    assert_uses_index(&db.conn, sql, "idx_media_assets_hash_pending");
    assert!(
        !plan(&db.conn, sql)
            .iter()
            .any(|d| d.contains("TEMP B-TREE")),
        "pending scan must not need a sort"
    );
}

#[test]
fn t_db_12_outbox_pending_scan_is_index_ordered() {
    let db = migrated_db();

    let sql = "SELECT event_id, asset_id, revision FROM search_outbox
               WHERE status = 'pending' ORDER BY priority, event_id LIMIT 100";
    assert_uses_index(&db.conn, sql, "idx_search_outbox_pending");
    assert!(
        !plan(&db.conn, sql)
            .iter()
            .any(|d| d.contains("TEMP B-TREE")),
        "outbox pending scan must not need a sort"
    );
}

#[test]
fn t_db_12_type_status_filter_uses_index() {
    let db = migrated_db();

    assert_uses_index(
        &db.conn,
        "SELECT asset_id FROM media_assets WHERE media_type = 'image' AND status = 'active' LIMIT 100",
        "idx_media_assets_type_status",
    );
}

#[test]
fn t_db_12_locations_by_volume_use_volume_index() {
    let db = migrated_db();

    assert_uses_index(
        &db.conn,
        "SELECT location_id FROM file_locations WHERE volume_id = 3",
        "idx_file_locations_volume",
    );
}

#[test]
fn t_db_12_relations_by_target_use_reverse_index() {
    let db = migrated_db();

    assert_uses_index(
        &db.conn,
        "SELECT from_asset_id FROM asset_relations WHERE to_asset_id = 3",
        "idx_asset_relations_to",
    );
}

#[test]
fn t_db_12_detector_flags_unindexed_query() {
    // Arrange: 대조군 — 인덱스 없는 컬럼 조건은 SCAN 으로 나와야 한다 (검사기가 눈먼 게 아님을 확인)
    let db = migrated_db();

    // Act
    let details = plan(
        &db.conn,
        "SELECT asset_id FROM media_assets WHERE file_size = 10",
    );

    // Assert
    assert!(details.iter().any(|d| d.starts_with("SCAN")), "{details:?}");
}
