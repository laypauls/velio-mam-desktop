//! `plugin_settings` 키 헬퍼 · `plugin_data` 정리 (T-pdb-07 ~ 10).
//!
//! 분류: Happy / Validation (빈·`:` 포함 ID, 손상 원문) / Boundary (범위 질의 경계, 배치 크기) / Business rule (설치된 것만 유지)

mod common;

use std::collections::BTreeSet;

use common::{count, insert_asset, migrated_db, seed_kv, seed_task};
use mam_db::plugin_data::{data_plugin_ids, purge_uninstalled, purge_uninstalled_with, tables_exist};
use mam_db::plugin_settings::{
    ENABLED, FEED_CURSOR, GRANTED, INSTALLED, delete_plugin, delete_prefixed, get, granted_to_raw, key,
    load_prefixed, parse_enabled, parse_granted, put, valid_id,
};
use rusqlite::Connection;

#[test]
fn t_pdb_07_keys_reject_empty_or_colon_ids_and_range_query_has_exact_bounds() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;

    // Act / Assert: ID 검증
    assert_eq!(key(ENABLED, "a.b").unwrap(), "plugin_enabled:a.b");
    assert!(key(ENABLED, "").is_err());
    assert!(key(ENABLED, "a:b").is_err());
    assert!(!valid_id("") && !valid_id("x:") && valid_id("x"));
    assert!(put(c, ENABLED, "a:b", "true").is_err());
    assert_eq!(count(c, "SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin_%'"), 0);

    // 범위 질의: 같은 접두어만, 이웃 키(접두어가 같지만 구분자가 다른 키)는 제외
    put(c, ENABLED, "p1", "true").unwrap();
    put(c, ENABLED, "p2", "false").unwrap();
    put(c, GRANTED, "p1", "[]").unwrap();
    c.execute("INSERT INTO settings (key, value) VALUES ('plugin_enabled', 'x'), ('plugin_enabled;z', 'x'), ('plugin_enabled:', 'x')", []).unwrap();
    let loaded = load_prefixed(c, ENABLED).unwrap();
    assert_eq!(loaded.keys().cloned().collect::<Vec<_>>(), ["p1", "p2"]);
    assert_eq!(load_prefixed(c, GRANTED).unwrap().len(), 1);

    // 지우기는 접두어 단위·플러그인 단위 모두 이웃을 건드리지 않는다
    put(c, INSTALLED, "p1", "{}").unwrap();
    put(c, FEED_CURSOR, "p1", "5").unwrap();
    delete_plugin(c, "p1").unwrap();
    assert!(load_prefixed(c, INSTALLED).unwrap().is_empty());
    assert_eq!(load_prefixed(c, ENABLED).unwrap().len(), 1);
    assert_eq!(delete_prefixed(c, ENABLED).unwrap(), 2, "p2 + the empty-id junk row inside the range");
    assert_eq!(count(c, "SELECT COUNT(*) FROM settings WHERE key IN ('plugin_enabled', 'plugin_enabled;z', 'plugin_enabled:')"), 2);
}

#[test]
fn t_pdb_08_plugin_keys_are_invisible_to_settings_load_and_values_parse_defensively() {
    // Arrange
    let db = migrated_db();
    let (before, _) = mam_db::settings::load(&db.conn).unwrap();
    put(&db.conn, GRANTED, "p", "[\"b\",\"a\"]").unwrap();
    put(&db.conn, ENABLED, "p", "true").unwrap();

    // Act
    let (after, warnings) = mam_db::settings::load(&db.conn).unwrap();

    // Assert: 설정 로드는 이 키를 모르는 키로 무시한다 (경고도 없다)
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(parse_enabled("true") && !parse_enabled("false") && !parse_enabled("1") && !parse_enabled("garbage"));
    assert_eq!(parse_granted("[\"b\",\"a\",1,null,\"a\"]"), set(&["a", "b"]));
    assert!(parse_granted("not json").is_empty() && parse_granted("{}").is_empty());
    assert_eq!(granted_to_raw(&set(&["z", "a"])), "[\"a\",\"z\"]");
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn seed(conn: &Connection, plugin: &str, assets: &[i64], global: usize) {
    for a in assets {
        seed_kv(conn, plugin, Some(*a), "k");
        seed_task(conn, plugin, "p", *a, 1, "pending");
    }
    for i in 0..global {
        seed_kv(conn, plugin, None, &format!("g{i}"));
    }
}

#[test]
fn t_pdb_09_purge_uninstalled_deletes_only_other_plugins_rows_in_batches_and_is_idempotent() {
    // Arrange: 설치됨 keep, 미설치 gone1·gone2 — 배치 크기 2 로 반복 경로를 탄다
    let db = migrated_db();
    let c = &db.conn;
    let assets: Vec<i64> = (0..5).map(|_| insert_asset(c)).collect();
    seed(c, "keep", &assets, 3);
    seed(c, "gone1", &assets, 4);
    seed(c, "gone2", &assets[..1], 0);
    assert_eq!(data_plugin_ids(c).unwrap(), set(&["gone1", "gone2", "keep"]));

    // Act
    let removed = purge_uninstalled_with(c, &set(&["keep"]), 2).unwrap();

    // Assert: gone1 = 5 + 4 + 5, gone2 = 1 + 1
    assert_eq!(removed, 14 + 2);
    assert_eq!(data_plugin_ids(c).unwrap(), set(&["keep"]));
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = 'keep'"), 8);
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = 'keep'"), 5);
    assert_eq!(purge_uninstalled(c, &set(&["keep"])).unwrap(), 0, "idempotent");
    // 아무것도 설치되지 않았으면 모두 지운다
    assert_eq!(purge_uninstalled(c, &BTreeSet::new()).unwrap(), 13);
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_kv") + count(c, "SELECT COUNT(*) FROM plugin_tasks"), 0);
}

#[test]
fn t_pdb_10_purge_is_a_no_op_before_v26_and_with_an_empty_db() {
    // Arrange: V25 이전 스키마 (표 없음)
    let old = Connection::open_in_memory().unwrap();
    assert!(!tables_exist(&old).unwrap());

    // Act / Assert
    assert_eq!(purge_uninstalled(&old, &BTreeSet::new()).unwrap(), 0);
    let db = migrated_db();
    assert!(tables_exist(&db.conn).unwrap());
    assert_eq!(purge_uninstalled(&db.conn, &BTreeSet::new()).unwrap(), 0);
}

#[test]
fn t_pdb_13_get_reads_one_row_and_fails_closed_for_missing_or_invalid_ids() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    put(c, ENABLED, "com.a", "true").unwrap();
    put(c, ENABLED, "com.b", "false").unwrap();
    // Act / Assert
    assert_eq!(get(c, ENABLED, "com.a").unwrap().as_deref(), Some("true"));
    assert_eq!(get(c, ENABLED, "com.b").unwrap().as_deref(), Some("false"));
    assert_eq!(get(c, GRANTED, "com.a").unwrap(), None, "다른 접두어는 별개");
    assert_eq!(get(c, ENABLED, "com.zzz").unwrap(), None);
    assert_eq!(get(c, ENABLED, "").unwrap(), None);
    assert_eq!(get(c, ENABLED, "a:b").unwrap(), None);
}
