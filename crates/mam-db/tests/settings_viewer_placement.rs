//! 플레이어 패널 위치 `viewerPlacement` 통합 테스트 (T-settings-vplace-01 ~ 06, 실제 SQLite).
//!
//! 분류: Happy (01 기본값 left, 02 왕복, 06 명시 저장된 right 유지) / Validation (03 잘못된 값 거부) / Business rule (04 손상 저장값은
//! 기본값 + 경고) / Boundary (05 구 버전 호환 — 모르는 키 행은 무시되고 이 키 행이 있어도 나머지 설정은 그대로)

mod common;

use common::migrated_db;
use mam_db::SettingsError;
use mam_db::settings;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn put_raw(conn: &Connection, key: &str, raw: &str) {
    conn.execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [key, raw]).unwrap();
}

fn placement_json(s: &mam_db::Settings) -> Value {
    serde_json::to_value(s).unwrap()["viewerPlacement"].clone()
}

#[test]
fn t_settings_vplace_01_defaults_to_left_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(placement_json(&s), json!("left"));
    assert!(warnings.is_empty());
}

#[test]
fn t_settings_vplace_02_left_and_right_round_trip_across_reload() {
    let mut db = migrated_db();
    for want in ["left", "right"] {
        let s = settings::update(&mut db.conn, &patch(json!({"viewerPlacement": want}))).unwrap();
        assert_eq!(placement_json(&s), json!(want));
        assert_eq!(placement_json(&settings::load(&db.conn).unwrap().0), json!(want));
    }
}

#[test]
fn t_settings_vplace_03_invalid_values_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    for bad in [json!("top"), json!(1), json!(null), json!(""), json!("Left"), json!(true), json!(["left"])] {
        let err = settings::update(&mut db.conn, &patch(json!({"viewerPlacement": bad.clone()})))
            .unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "viewerPlacement"), "{bad}");
    }
    let rows: i64 = db.conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn t_settings_vplace_04_corrupt_stored_value_falls_back_to_left_with_a_warning() {
    for raw in ["\"top\"", "1", "null", "not json", "\"\""] {
        let db = migrated_db();
        put_raw(&db.conn, "viewer_placement", raw);

        let (s, warnings) = settings::load(&db.conn).unwrap();

        assert_eq!(placement_json(&s), json!("left"), "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}: {warnings:?}");
    }
}

#[test]
fn t_settings_vplace_05_unknown_key_rows_are_ignored_on_read_like_an_older_app() {
    let db = migrated_db();
    // 구 버전 앱은 DEFS 에 없는 키 행을 만나는 것과 같다 (아래 키는 현 DEFS 에도 없는 가상 키)
    put_raw(&db.conn, "future_setting_xyz", "\"left\"");
    put_raw(&db.conn, "close_to_tray", "false");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(!s.close_to_tray, "known keys still apply");
    assert_eq!(placement_json(&s), json!("left"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn t_settings_vplace_06_explicit_right_stored_stays_right_without_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "viewer_placement", "\"right\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(placement_json(&s), json!("right"));
    assert!(warnings.is_empty(), "{warnings:?}");
}
