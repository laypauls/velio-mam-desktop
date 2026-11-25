//! 외부 자막 설정 `subtitleVisible` / `subtitleDefaultLanguage` 통합 테스트 (T-settings-sub-01 ~ 04).
//!
//! 분류: Happy (01 기본값, 02 왕복) / Validation (03 잘못된 패치) / Business rule (04 손상 저장값은 기본값 + 경고)

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
fn t_settings_sub_01_defaults_are_visible_and_korean_with_contract_names() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert!(s.subtitle_visible);
    assert_eq!(s.subtitle_default_language.as_deref(), Some("ko"));
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["subtitleVisible"], json!(true));
    assert_eq!(v["subtitleDefaultLanguage"], json!("ko"));
    assert!(warnings.is_empty());
}

#[test]
fn t_settings_sub_02_round_trips_every_language_and_null() {
    let mut db = migrated_db();
    for lang in [
        json!("ja"),
        json!("en"),
        json!("zh"),
        json!("ko"),
        Value::Null,
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"subtitleDefaultLanguage": lang.clone()})),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&s).unwrap()["subtitleDefaultLanguage"],
            lang
        );
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
    for visible in [false, true] {
        let s =
            settings::update(&mut db.conn, &patch(json!({"subtitleVisible": visible}))).unwrap();
        assert_eq!(s.subtitle_visible, visible);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_settings_sub_03_invalid_updates_are_rejected_and_nothing_is_stored() {
    let mut db = migrated_db();
    for bad in [
        json!("KO"),
        json!("fr"),
        json!(""),
        json!(1),
        json!(true),
        json!(["ko"]),
        json!("ko "),
    ] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"subtitleDefaultLanguage": bad.clone()})),
        )
        .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "subtitleDefaultLanguage"),
            "{bad}"
        );
    }
    for bad in [json!("true"), json!(1), Value::Null, json!("ko")] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"subtitleVisible": bad.clone()})),
        )
        .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "subtitleVisible"),
            "{bad}"
        );
    }
    // 한 패치 안의 유효한 값도 함께 버려진다 (한 트랜잭션)
    let err = settings::update(
        &mut db.conn,
        &patch(json!({"subtitleVisible": false, "subtitleDefaultLanguage": "xx"})),
    )
    .unwrap_err();
    assert!(matches!(err, SettingsError::Invalid(_)));
    assert_eq!(count_rows(&db.conn), 0);
}

#[test]
fn t_settings_sub_04_corrupt_stored_values_fall_back_to_defaults_with_warnings() {
    let db = migrated_db();
    put_raw(&db.conn, "subtitle_default_language", "\"KO\"");
    put_raw(&db.conn, "subtitle_visible", "1");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.subtitle_default_language.as_deref(), Some("ko"));
    assert!(s.subtitle_visible);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    // 경고에는 설정 키 이름만 싣고 저장된 값은 싣지 않는다
    assert!(warnings.iter().all(|w| !w.contains("KO")));
    // 모르는 언어 문자열(`fr`)도 같다 / 저장된 null 은 유효하다 (기본 언어 없음)
    let db = migrated_db();
    put_raw(&db.conn, "subtitle_default_language", "\"fr\"");
    assert_eq!(
        settings::load(&db.conn)
            .unwrap()
            .0
            .subtitle_default_language
            .as_deref(),
        Some("ko")
    );
    let db = migrated_db();
    put_raw(&db.conn, "subtitle_default_language", "null");
    let (s, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(s.subtitle_default_language, None);
    assert!(warnings.is_empty());
}
