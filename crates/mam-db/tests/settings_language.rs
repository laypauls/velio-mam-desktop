//! UI 표시 언어 설정 `language` 통합 테스트 (T-locale-10 ~ 13).
//!
//! 분류: Happy (10 기본값 en, 11 en/ko/zh-TW 왕복·재로드, 14 저장된 ko 유지) / Validation (12 잘못된 패치 거부, 13 손상된 저장값 -> en + 경고).

mod common;

use common::migrated_db;
use mam_db::SettingsError;
use mam_db::settings;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn t_locale_10_default_is_en_when_key_absent() {
    // Arrange
    let db = migrated_db();

    // Act
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert_eq!(loaded.language, "en");
    assert!(warnings.is_empty());
    assert_eq!(serde_json::to_value(&loaded).unwrap()["language"], "en");
}

#[test]
fn t_locale_14_stored_ko_is_kept_for_existing_users() {
    // Arrange: 기본값이 en 으로 바뀌기 전에 ko 를 저장한 사용자
    let db = migrated_db();
    db.conn
        .execute(
            "INSERT INTO settings (key, value) VALUES ('language', ?1)",
            [r#""ko""#],
        )
        .unwrap();

    // Act
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert_eq!(loaded.language, "ko");
    assert!(warnings.is_empty());
}

#[test]
fn t_locale_11_valid_codes_round_trip_and_persist() {
    for code in ["en", "zh-TW"] {
        // Arrange
        let mut db = migrated_db();

        // Act
        let updated = settings::update(&mut db.conn, &patch(json!({"language": code}))).unwrap();
        drop(db.conn);
        let reopened = mam_db::open(&db.db_path).unwrap();
        let (after, warnings) = settings::load(&reopened).unwrap();

        // Assert
        assert_eq!(updated.language, code);
        assert_eq!(after.language, code);
        assert!(warnings.is_empty());
    }
}

#[test]
fn t_locale_12_invalid_patch_is_rejected_and_nothing_is_stored() {
    // Arrange
    let mut db = migrated_db();

    for bad in [
        json!(""),
        json!("EN_us"),
        json!("../x"),
        json!("a\\b"),
        json!(5),
        json!(null),
        json!(["en"]),
    ] {
        // Act
        let err = settings::update(&mut db.conn, &patch(json!({"language": bad}))).unwrap_err();

        // Assert
        assert!(
            matches!(err, SettingsError::Invalid(f) if f == "language"),
            "{bad}"
        );
    }
    let n: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key = 'language'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_locale_13_corrupt_stored_value_falls_back_to_en_with_warning() {
    for raw in [r#""EN_us""#, r#""../x""#, "5", "not json"] {
        // Arrange
        let db = migrated_db();
        db.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES ('language', ?1)",
                [raw],
            )
            .unwrap();

        // Act
        let (loaded, warnings) = settings::load(&db.conn).unwrap();

        // Assert
        assert_eq!(loaded.language, "en", "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}");
        assert!(warnings[0].contains("language"), "{raw}");
    }
}
