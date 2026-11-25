//! 플러그인 레지스트리 (`plugins`·`plugin_providers`) — 정수 키 발급·제약·읽기 전용 조회 (T-pref-01·02·03·05·06).
//!
//! 분류: Happy (ensure 멱등·provider 키) / Validation (빈·`:` ID, UNIQUE, FK) / Business rule (삭제 후 같은 키 재사용, 레지스트리 행 유지) /
//! Boundary (레지스트리에 없는 플러그인을 읽는 경로). 실제 SQLite 에 마이그레이션을 적용해 쓴다.

mod common;

use std::collections::BTreeSet;

use common::{count, insert_asset, migrated_db, seed_kv, seed_task};
use mam_db::plugin_data::{data_plugin_ids, delete_plugin_batch, purge_uninstalled};
use mam_db::plugin_kv;
use mam_db::plugin_registry::{all, ensure, ensure_provider, lookup, lookup_provider};
use mam_db::plugin_settings::PutError;
use mam_db::plugin_tasks::{self, counts_by_plugin, pending_count, pending_for_asset, pending_page, retry_failed};

const A: &str = "com.example.a";
const B: &str = "com.example.b";

#[test]
fn t_pref_01_ensure_is_idempotent_lookup_never_creates_and_provider_refs_are_per_plugin() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;

    // Act
    let unknown = lookup(c, A).unwrap();
    let first = ensure(c, A).unwrap();
    let second = ensure(c, A).unwrap();
    let other = ensure(c, B).unwrap();
    let (pa, pb) = (ensure_provider(c, first, "meta").unwrap(), ensure_provider(c, other, "meta").unwrap());

    // Assert
    assert_eq!(unknown, None);
    assert_eq!(first, second, "같은 ID 는 같은 키");
    assert_ne!(first, other);
    assert_ne!(pa, pb, "같은 provider 이름도 플러그인마다 다른 키");
    assert_eq!(ensure_provider(c, first, "meta").unwrap(), pa, "provider 도 멱등");
    assert_eq!(lookup(c, B).unwrap(), Some(other));
    assert_eq!(lookup_provider(c, first, "meta").unwrap(), Some(pa));
    assert_eq!(lookup_provider(c, first, "nope").unwrap(), None);
    assert_eq!(all(c).unwrap(), vec![(first, A.to_owned()), (other, B.to_owned())], "plugin_id 순");
}

#[test]
fn t_pref_02_invalid_ids_are_rejected_by_rust_and_by_check_and_duplicates_by_unique() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;

    // Act / Assert: Rust 검사 (행을 만들지 않는다)
    assert!(matches!(ensure(c, ""), Err(PutError::Id(_))));
    assert!(matches!(ensure(c, "a:b"), Err(PutError::Id(_))));
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugins"), 0);
    // CHECK (Rust 검사를 우회한 원시 삽입)
    assert!(c.execute("INSERT INTO plugins (plugin_id) VALUES ('')", []).is_err());
    assert!(c.execute("INSERT INTO plugins (plugin_id) VALUES ('a:b')", []).is_err());
    // UNIQUE
    c.execute("INSERT INTO plugins (plugin_id) VALUES ('dup')", []).unwrap();
    assert!(c.execute("INSERT INTO plugins (plugin_id) VALUES ('dup')", []).is_err());
    // provider: 빈 이름·같은 (plugin, name) 중복
    let r = ensure(c, A).unwrap();
    assert!(ensure_provider(c, r, "").is_err());
    c.execute("INSERT INTO plugin_providers (plugin_ref, name) VALUES (?1, 'x')", [r]).unwrap();
    assert!(c.execute("INSERT INTO plugin_providers (plugin_ref, name) VALUES (?1, 'x')", [r]).is_err());
    assert!(ensure_provider(c, 9999, "y").is_err(), "없는 plugin_ref FK");
}

#[test]
fn t_pref_03_foreign_keys_reject_unknown_plugin_refs_and_cross_plugin_provider_refs() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    let (a, b) = (ensure(c, A).unwrap(), ensure(c, B).unwrap());
    let (_va, vb) = (ensure_provider(c, a, "p").unwrap(), ensure_provider(c, b, "p").unwrap());
    let insert_task = |plugin_ref: i64, provider_ref: i64| {
        c.execute(
            "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at) VALUES (?1, ?2, ?3, 1, 0)",
            (plugin_ref, provider_ref, asset),
        )
    };

    // Act / Assert
    assert!(c.execute("INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (999, NULL, 'k', '1', 0)", []).is_err(), "unknown plugin_ref");
    assert!(insert_task(a, vb).is_err(), "A 의 task 가 B 의 provider 를 가리킬 수 없다 (복합 FK)");
    assert!(insert_task(999, vb).is_err());
    assert!(insert_task(b, vb).is_ok());
}

#[test]
fn t_pref_05_deleting_a_plugins_data_keeps_the_registry_row_and_the_same_ref_is_reused() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let asset = insert_asset(c);
    seed_kv(c, A, Some(asset), "k");
    seed_task(c, A, "meta", asset, 1, "pending");
    let (ref_before, provider_before) = (lookup(c, A).unwrap().unwrap(), {
        let r = lookup(c, A).unwrap().unwrap();
        lookup_provider(c, r, "meta").unwrap().unwrap()
    });

    // Act
    let mut removed = 0;
    loop {
        let n = delete_plugin_batch(c, A, 1).unwrap();
        if n == 0 {
            break;
        }
        removed += n;
    }
    plugin_kv::upsert(c, A, None, "again", "1", 1).unwrap();

    // Assert
    assert_eq!(removed, 2);
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_tasks"), 0);
    assert_eq!(lookup(c, A).unwrap(), Some(ref_before), "레지스트리 행은 지우지 않는다 — 같은 ID 는 같은 키");
    assert_eq!(ensure_provider(c, ref_before, "meta").unwrap(), provider_before);
    assert_eq!(plugin_kv::get(c, A, None, "again").unwrap().as_deref(), Some("1"));
    assert_eq!(plugin_kv::get(c, A, Some(asset), "k").unwrap(), None, "남은 행 없음");
}

#[test]
fn t_pref_06_readers_for_a_plugin_without_a_registry_row_return_empty_and_create_nothing() {
    // Arrange
    let mut db = migrated_db();
    let asset = insert_asset(&db.conn);
    seed_task(&db.conn, B, "meta", asset, 1, "pending"); // 다른 플러그인 데이터는 보이면 안 된다
    let registry_rows = count(&db.conn, "SELECT COUNT(*) FROM plugins");
    let c = &db.conn;

    // Act / Assert: KV
    assert_eq!(plugin_kv::get(c, A, None, "k").unwrap(), None);
    assert_eq!(plugin_kv::get(c, A, Some(asset), "k").unwrap(), None);
    assert_eq!(plugin_kv::row_cost(c, A, None, "k").unwrap(), None);
    assert!(plugin_kv::list_keys(c, A, None, Some("p"), None, 10).unwrap().is_empty());
    assert!(plugin_kv::list_keys(c, A, Some(asset), None, Some("c"), 10).unwrap().is_empty());
    assert_eq!(plugin_kv::sum_cost(c, A).unwrap(), 0);
    assert!(!plugin_kv::delete(c, A, None, "k").unwrap());
    // 작업
    assert_eq!(pending_count(c, A).unwrap(), 0);
    assert_eq!(plugin_tasks::failed_count(c, A).unwrap(), 0);
    assert!(pending_page(c, A, 0, 10).unwrap().is_empty());
    assert_eq!(counts_by_plugin(c, &[A]).unwrap()[0].pending, 0);
    assert_eq!(pending_for_asset(c, asset).unwrap().len(), 1, "B 의 대기 작업만 (문자열 그대로)");
    assert!(plugin_tasks::with_immediate_tx(&mut db.conn, |tx| retry_failed(tx, A, 0)).unwrap().is_empty());
    // 정리
    assert_eq!(delete_plugin_batch(&db.conn, A, 10).unwrap(), 0);
    assert_eq!(purge_uninstalled(&db.conn, &BTreeSet::from([B.to_owned()])).unwrap(), 0);
    assert_eq!(data_plugin_ids(&db.conn).unwrap(), BTreeSet::from([B.to_owned()]));
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugins"), registry_rows, "읽기는 레지스트리 행을 만들지 않는다");
}

#[test]
fn t_pref_13_pending_for_asset_returns_the_plugin_and_provider_strings_that_were_fed() {
    // Arrange
    let mut db = migrated_db();
    let asset = insert_asset(&db.conn);
    db.conn.execute("INSERT INTO search_outbox (asset_id, revision, event_type, created_at) VALUES (?1, 3, 'upsert', 0)", [asset]).unwrap();
    let upto = plugin_tasks::max_event_id(&db.conn).unwrap();

    // Act
    plugin_tasks::with_immediate_tx(&mut db.conn, |tx| {
        plugin_tasks::feed_batch(tx, A, &[("meta", &["image"]), ("tags", &[])], 0, upto, 7)?;
        plugin_tasks::feed_batch(tx, B, &[("meta", &[])], 0, upto, 7)
    })
    .unwrap();
    let rows = pending_for_asset(&db.conn, asset).unwrap();

    // Assert
    let got: Vec<(&str, &str, i64)> = rows.iter().map(|t| (t.plugin_id.as_str(), t.provider.as_str(), t.target_revision)).collect();
    assert_eq!(got, [(A, "meta", 3), (A, "tags", 3), (B, "meta", 3)], "task_id 순 = 공급 순");
}

#[test]
fn t_pref_02_feed_with_an_invalid_plugin_id_is_rejected_without_creating_rows() {
    // Arrange
    let mut db = migrated_db();
    let asset = insert_asset(&db.conn);
    db.conn.execute("INSERT INTO search_outbox (asset_id, revision, event_type, created_at) VALUES (?1, 1, 'upsert', 0)", [asset]).unwrap();

    // Act
    let bad = plugin_tasks::with_immediate_tx(&mut db.conn, |tx| plugin_tasks::feed_batch(tx, "a:b", &[("m", &[])], 0, 1, 0));
    let upsert = plugin_kv::upsert(&db.conn, "", None, "k", "1", 0);

    // Assert
    assert!(matches!(bad, Err(rusqlite::Error::InvalidParameterName(_))), "{bad:?}");
    assert!(matches!(upsert, Err(rusqlite::Error::InvalidParameterName(_))), "{upsert:?}");
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugins") + count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks"), 0);
}
