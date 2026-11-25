//! 플러그인 KV 질의 (T-pdb-kv-*) — 실제 SQLite(마이그레이션 적용).
//!
//! 분류: Happy(왕복·upsert) / Boundary(prefix·커서·유니코드) / Business rule(네임스페이스·전역/자산 독립) / Performance(EXPLAIN QUERY PLAN)

mod common;

use common::{insert_asset, migrated_db};
use mam_db::plugin_kv::*;
use rusqlite::Connection;

const A: &str = "com.example.a";
const B: &str = "com.example.b";

fn seed(conn: &Connection, plugin: &str, asset: Option<i64>, keys: &[&str]) {
    for k in keys {
        upsert(conn, plugin, asset, k, "1", 5).unwrap();
    }
}

#[test]
fn t_pdb_kv_02_roundtrip_upsert_delete_global_and_asset_are_independent() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    // Act
    upsert(c, A, None, "k", r#"{"v":"가"}"#, 1).unwrap();
    upsert(c, A, Some(asset), "k", "2", 1).unwrap();
    upsert(c, A, None, "k", r#"{"v":"나"}"#, 2).unwrap();
    // Assert: 덮어쓰기는 1행, 전역·자산 키는 별개
    assert_eq!(get(c, A, None, "k").unwrap().as_deref(), Some(r#"{"v":"나"}"#));
    assert_eq!(get(c, A, Some(asset), "k").unwrap().as_deref(), Some("2"));
    assert_eq!(get(c, A, None, "nope").unwrap(), None);
    assert!(delete(c, A, None, "k").unwrap());
    assert!(!delete(c, A, None, "k").unwrap(), "멱등");
    assert_eq!(get(c, A, Some(asset), "k").unwrap().as_deref(), Some("2"), "자산 키는 남는다");
}

#[test]
fn t_pdb_kv_03_namespace_is_enforced_in_every_query() {
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    seed(c, A, None, &["x", "y"]);
    seed(c, A, Some(asset), &["x"]);
    assert_eq!(get(c, B, None, "x").unwrap(), None);
    assert_eq!(get(c, B, Some(asset), "x").unwrap(), None);
    assert!(list_keys(c, B, None, None, None, 10).unwrap().is_empty());
    assert!(!delete(c, B, None, "x").unwrap());
    assert_eq!(get(c, A, None, "x").unwrap().as_deref(), Some("1"));
    // 같은 키 이름이라도 서로 다른 행
    upsert(c, B, None, "x", "9", 1).unwrap();
    assert_eq!(get(c, A, None, "x").unwrap().as_deref(), Some("1"));
    assert_eq!(sum_cost(c, B).unwrap(), row_cost(c, B, None, "x").unwrap().unwrap());
}

#[test]
fn t_pdb_kv_04_list_keys_keyset_pages_have_no_gaps_or_duplicates() {
    let db = migrated_db();
    let c = &db.conn;
    let keys: Vec<String> = (0..450).map(|i| format!("k{i:04}")).collect();
    for k in &keys {
        upsert(c, A, None, k, "1", 1).unwrap();
    }
    let mut got = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = list_keys(c, A, None, None, cursor.as_deref(), 200).unwrap();
        if page.is_empty() {
            break;
        }
        cursor = page.last().cloned();
        got.extend(page);
    }
    assert_eq!(got, keys);
}

#[test]
fn t_pdb_kv_05_prefix_is_a_literal_range_not_a_pattern() {
    // Arrange: LIKE 와 달리 `%`·`_` 는 문자 그대로여야 한다
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    for scope in [None, Some(asset)] {
        seed(c, A, scope, &["a%b", "a%c", "axb", "a_b", "azz", "ab", "가나", "가다", "각", "나"]);
        // Act / Assert
        assert_eq!(list_keys(c, A, scope, Some("a%"), None, 50).unwrap(), ["a%b", "a%c"]);
        assert_eq!(list_keys(c, A, scope, Some("a_"), None, 50).unwrap(), ["a_b"]);
        assert_eq!(list_keys(c, A, scope, Some("가"), None, 50).unwrap(), ["가나", "가다"]);
        assert_eq!(list_keys(c, A, scope, Some("a%"), Some("a%b"), 50).unwrap(), ["a%c"], "커서는 접두사 범위 안에서 이어진다");
        assert_eq!(list_keys(c, A, scope, Some("a%"), Some("a"), 50).unwrap(), ["a%b", "a%c"], "커서가 범위 앞이면 접두사부터");
        assert!(list_keys(c, A, scope, Some("a%"), Some("zzz"), 50).unwrap().is_empty());
        assert_eq!(list_keys(c, A, scope, Some("a%"), None, 1).unwrap(), ["a%b"]);
        assert!(list_keys(c, A, scope, Some("nomatch"), None, 50).unwrap().is_empty());
        // 빈 접두사 = 접두사 없음
        assert_eq!(list_keys(c, A, scope, Some(""), None, 100).unwrap().len(), 10);
    }
}

#[test]
fn t_pdb_kv_06_costs_sum_both_scopes_and_count_bytes_not_chars() {
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    upsert(c, A, None, "가", "\"나\"", 1).unwrap(); // 키 3바이트 + 값 5바이트
    upsert(c, A, Some(asset), "k", "1", 1).unwrap(); // 1 + 1
    assert_eq!(row_cost(c, A, None, "가").unwrap(), Some(3 + 5 + ROW_OVERHEAD));
    assert_eq!(row_cost(c, A, None, "없음").unwrap(), None);
    assert_eq!(sum_cost(c, A).unwrap(), (3 + 5 + ROW_OVERHEAD) + (1 + 1 + ROW_OVERHEAD));
    assert_eq!(sum_cost(c, B).unwrap(), 0);
    // 자산 삭제(CASCADE)는 합계를 줄인다
    c.execute("DELETE FROM media_assets WHERE asset_id = ?1", [asset]).unwrap();
    assert_eq!(sum_cost(c, A).unwrap(), 3 + 5 + ROW_OVERHEAD);
}

#[test]
fn t_pdb_kv_07_count_asset_tags_counts_only_that_asset() {
    let db = migrated_db();
    let c = &db.conn;
    let (a1, a2) = (insert_asset(c), insert_asset(c));
    c.execute("INSERT INTO tags (name, name_key) VALUES ('x','x'), ('y','y')", []).unwrap();
    c.execute("INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, 0), (?1, 2, 0), (?2, 1, 0)", [a1, a2]).unwrap();
    assert_eq!(count_asset_tags(c, a1).unwrap(), 2);
    assert_eq!(count_asset_tags(c, a2).unwrap(), 1);
    assert_eq!(count_asset_tags(c, 999).unwrap(), 0);
}

fn plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let n = sql.matches('?').count();
    let rows: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(std::iter::repeat_n(1i64, n)), |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.join(" | ")
}

#[test]
fn t_pdb_kv_08_list_and_cost_queries_use_the_partial_indexes_without_temp_btree() {
    // Arrange / Act / Assert: 접두사 있음·없음 × 하한 열림·닫힘 × 전역·자산 (계약: SEARCH, SCAN·임시 B-tree 없음)
    let db = migrated_db();
    let c = &db.conn;
    for global in [true, false] {
        let index = if global { "uq_plugin_kv_global" } else { "uq_plugin_kv_asset" };
        for lo_inclusive in [false, true] {
            for has_hi in [false, true] {
                let p = plan(c, &list_sql(global, lo_inclusive, has_hi));
                assert!(p.contains("SEARCH") && p.contains(index), "{global} {lo_inclusive} {has_hi}: {p}");
                assert!(!p.contains("SCAN") && !p.contains("TEMP B-TREE"), "{global} {lo_inclusive} {has_hi}: {p}");
                if has_hi {
                    assert!(p.contains("key>? AND key<?") || p.contains("key>=? AND key<?"), "범위 상·하한 모두 인덱스에 쓰인다: {p}");
                }
            }
        }
    }
    for (sql, index) in [
        ("SELECT COALESCE(SUM(length(CAST(key AS BLOB))),0) FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NULL", "uq_plugin_kv_global"),
        ("SELECT COALESCE(SUM(length(CAST(key AS BLOB))),0) FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NOT NULL", "uq_plugin_kv_asset"),
        ("SELECT length(CAST(key AS BLOB)) FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id = ?2 AND key = ?3", "uq_plugin_kv_asset"),
        ("DELETE FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NULL AND key = ?2", "uq_plugin_kv_global"),
        ("DELETE FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id = ?2 AND key = ?3", "uq_plugin_kv_asset"),
        ("SELECT COUNT(*) FROM asset_tags WHERE asset_id = ?1", "sqlite_autoindex_asset_tags_1"),
    ] {
        let p = plan(c, sql);
        assert!(p.contains("SEARCH") && p.contains(index) && !p.contains("SCAN"), "{sql}: {p}");
    }
}
