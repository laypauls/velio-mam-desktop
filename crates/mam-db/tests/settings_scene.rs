//! 장면전환점 설정 3키 `sceneCutsVisible` / `sceneDetectAuto` / `sceneDetectSensitivity` (T-scene-15).
//!
//! 분류: Happy (기본값·왕복) / Validation (잘못된 패치 거부) / Business rule (손상 저장값은 기본값 + 경고, 단일 키 로더)

mod common;

use common::migrated_db;
use mam_db::scene_cuts::{load_scene_detect_auto, load_scene_sensitivity};
use mam_db::{SceneSensitivity, SettingsError, settings};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn put_raw(conn: &Connection, key: &str, raw: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, raw],
    )
    .unwrap();
}

#[test]
fn t_scene_15_defaults_are_visible_on_auto_off_and_medium_with_contract_names() {
    // Arrange / Act
    let db = migrated_db();
    let (s, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert!(s.scene_cuts_visible);
    assert!(!s.scene_detect_auto);
    assert_eq!(s.scene_detect_sensitivity, SceneSensitivity::Medium);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["sceneCutsVisible"], json!(true));
    assert_eq!(v["sceneDetectAuto"], json!(false));
    assert_eq!(v["sceneDetectSensitivity"], json!("medium"));
    assert!(warnings.is_empty());
    // 로더도 같은 기본값
    assert!(!load_scene_detect_auto(&db.conn).unwrap());
    assert_eq!(load_scene_sensitivity(&db.conn).unwrap(), SceneSensitivity::Medium);
}

#[test]
fn t_scene_15_values_round_trip_and_the_single_key_loaders_see_them() {
    // Arrange
    let mut db = migrated_db();

    // Act / Assert
    for (sens, name) in [
        (SceneSensitivity::Low, "low"),
        (SceneSensitivity::High, "high"),
        (SceneSensitivity::Medium, "medium"),
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"sceneDetectSensitivity": name, "sceneDetectAuto": true, "sceneCutsVisible": false})),
        )
        .unwrap();
        assert_eq!(s.scene_detect_sensitivity, sens);
        assert!(s.scene_detect_auto && !s.scene_cuts_visible);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
        assert_eq!(load_scene_sensitivity(&db.conn).unwrap(), sens);
        assert!(load_scene_detect_auto(&db.conn).unwrap());
        assert_eq!(sens.as_str(), name);
    }
    let off = settings::update(&mut db.conn, &patch(json!({"sceneDetectAuto": false, "sceneCutsVisible": true}))).unwrap();
    assert!(!off.scene_detect_auto && off.scene_cuts_visible);
    assert!(!load_scene_detect_auto(&db.conn).unwrap());
}

#[test]
fn t_scene_15_invalid_updates_are_rejected_and_nothing_is_stored() {
    // Arrange
    let mut db = migrated_db();
    let rows_before: i64 = db.conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();

    // Act / Assert
    for bad in [json!("extreme"), json!("Medium"), json!("HIGH"), json!(""), json!(1), json!(true), Value::Null, json!(["low"])] {
        let err = settings::update(&mut db.conn, &patch(json!({"sceneDetectSensitivity": bad.clone()}))).unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "sceneDetectSensitivity"), "{bad}");
    }
    for field in ["sceneDetectAuto", "sceneCutsVisible"] {
        for bad in [json!("true"), json!(1), json!(0), Value::Null, json!("on")] {
            let err = settings::update(&mut db.conn, &patch(json!({field: bad.clone()}))).unwrap_err();
            assert!(matches!(&err, SettingsError::Invalid(f) if f == field), "{field} {bad}");
        }
    }
    // 한 키라도 잘못되면 같은 패치의 나머지도 저장되지 않는다
    let err = settings::update(&mut db.conn, &patch(json!({"sceneDetectAuto": true, "sceneDetectSensitivity": "x"}))).unwrap_err();
    assert!(matches!(err, SettingsError::Invalid(_)));
    let rows_after: i64 = db.conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();
    assert_eq!(rows_after, rows_before);
    assert!(!load_scene_detect_auto(&db.conn).unwrap());
}

#[test]
fn t_scene_15_corrupt_stored_values_fall_back_to_defaults_with_key_name_only_warnings() {
    // Arrange: 손상된 저장값 (JSON 아님·타입 불일치·모르는 열거값)
    let db = migrated_db();
    put_raw(&db.conn, "scene_cuts_visible", "\"yes\"");
    put_raw(&db.conn, "scene_detect_auto", "1");
    put_raw(&db.conn, "scene_detect_sensitivity", "\"ultra\"");

    // Act
    let (s, warnings) = settings::load(&db.conn).unwrap();

    // Assert: 보기는 켬(기본), 자동은 끔(안전 기본), 민감도는 보통
    assert!(s.scene_cuts_visible);
    assert!(!s.scene_detect_auto);
    assert_eq!(s.scene_detect_sensitivity, SceneSensitivity::Medium);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("ultra")), "값은 싣지 않는다");
    // 단일 키 로더도 같은 폴백 — 특히 자동 추출은 켜지지 않는다
    assert!(!load_scene_detect_auto(&db.conn).unwrap());
    assert_eq!(load_scene_sensitivity(&db.conn).unwrap(), SceneSensitivity::Medium);
    for raw in ["not json", "\"true\"", "null", "2"] {
        put_raw(&db.conn, "scene_detect_auto", raw);
        assert!(!load_scene_detect_auto(&db.conn).unwrap(), "{raw}");
    }
    for raw in ["not json", "\"LOW\"", "3", "null"] {
        put_raw(&db.conn, "scene_detect_sensitivity", raw);
        assert_eq!(load_scene_sensitivity(&db.conn).unwrap(), SceneSensitivity::Medium, "{raw}");
    }
}
