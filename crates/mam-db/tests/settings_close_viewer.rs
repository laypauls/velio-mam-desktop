//! 플레이어 패널 닫기 단축키 `closeViewerShortcut` 통합 테스트 (T-settings-closekey-01 ~ 04).
//!
//! 분류: Happy (01 기본값 Q, 02 왕복) / Validation (03 잘못된 패치) / Business rule (04 손상 저장값은 기본값 + 경고)

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
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)",
        [key, raw],
    )
    .unwrap();
}

fn count_rows(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn t_settings_closekey_01_defaults_to_q_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(s.close_viewer_shortcut.as_deref(), Some("Q"));
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["closeViewerShortcut"], json!("Q"));
    assert!(warnings.is_empty());
}

#[test]
fn t_settings_closekey_02_round_trips_valid_values_including_null() {
    let mut db = migrated_db();
    for want in [
        json!("Q"),
        json!("Ctrl+Shift+W"),
        json!("Escape"),
        json!("a".repeat(20)),
        Value::Null,
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"closeViewerShortcut": want.clone()})),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&s).unwrap()["closeViewerShortcut"],
            want
        );
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_settings_closekey_03_invalid_values_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    for bad in [
        json!(""),
        json!("Ctrl+"),
        json!("A B"),
        json!("Shift+Ctrl+Q"),
        json!("a".repeat(21)),
        json!(1),
        json!(true),
        json!(["Q"]),
    ] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"closeViewerShortcut": bad.clone()})),
        )
        .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "closeViewerShortcut"),
            "{bad}"
        );
    }
    assert_eq!(count_rows(&db.conn), 0);
}

#[test]
fn t_settings_closekey_04_corrupt_stored_value_falls_back_to_q_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "close_viewer_shortcut", "\"Shift+Ctrl+Q\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.close_viewer_shortcut.as_deref(), Some("Q"));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("Shift+Ctrl")));
}

#[test]
fn t_settings_closekey_05_is_independent_from_hide_shortcut() {
    let mut db = migrated_db();

    let s = settings::update(&mut db.conn, &patch(json!({"closeViewerShortcut": null}))).unwrap();

    assert_eq!(s.close_viewer_shortcut, None);
    assert_eq!(s.hide_shortcut.as_deref(), Some("Escape"));
}
