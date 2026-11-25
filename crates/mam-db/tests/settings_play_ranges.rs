//! 재생 구간·열 때 재생 설정 4종 (T-settings-ranges-01 ~ 04).
//!
//! 분류: Happy (01 기본값, 02 왕복) / Validation (03 손상 저장값 대체, 04 잘못된 패치 거부·무저장)

mod common;

use common::migrated_db;
use mam_db::settings;
use mam_db::{PlayRangeKeys, SettingsError};
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn t_settings_ranges_01_defaults() {
    // Arrange + Act
    let db = migrated_db();
    let (s, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert!(!s.media_open_autoplay && !s.play_ranges_only && !s.collection_play_ranges_only);
    assert_eq!(s.play_range_keys, PlayRangeKeys::Brackets);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["mediaOpenAutoplay"], json!(false));
    assert_eq!(v["playRangeKeys"], json!("brackets"));
    assert_eq!(v["playRangesOnly"], json!(false));
    assert_eq!(v["collectionPlayRangesOnly"], json!(false));
    assert!(warnings.is_empty());
}

#[test]
fn t_settings_ranges_02_round_trip() {
    // Arrange
    let mut db = migrated_db();

    // Act
    let after = settings::update(
        &mut db.conn,
        &patch(json!({
            "mediaOpenAutoplay": true, "playRangeKeys": "premiere",
            "playRangesOnly": true, "collectionPlayRangesOnly": true
        })),
    )
    .unwrap();
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert_eq!(after, loaded);
    assert!(loaded.media_open_autoplay && loaded.play_ranges_only && loaded.collection_play_ranges_only);
    assert_eq!(loaded.play_range_keys, PlayRangeKeys::Premiere);
    assert_eq!(serde_json::to_value(&loaded).unwrap()["playRangeKeys"], json!("premiere"));
    assert!(warnings.is_empty());
    let back = settings::update(
        &mut db.conn,
        &patch(json!({"playRangeKeys": "brackets", "playRangesOnly": false})),
    )
    .unwrap();
    assert_eq!(back.play_range_keys, PlayRangeKeys::Brackets);
    assert!(!back.play_ranges_only && back.media_open_autoplay);
}

#[test]
fn t_settings_ranges_03_corrupt_stored_values_fall_back_with_warning_without_value() {
    for (key, raw) in [
        ("play_range_keys", "\"Brackets\""),
        ("play_range_keys", "1"),
        ("play_range_keys", "null"),
        ("play_range_keys", "\"x\""),
        ("play_range_keys", "not json"),
        ("media_open_autoplay", "1"),
        ("play_ranges_only", "\"true\""),
        ("collection_play_ranges_only", "null"),
    ] {
        // Arrange
        let db = migrated_db();
        db.conn
            .execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [key, raw])
            .unwrap();

        // Act
        let (s, warnings) = settings::load(&db.conn).unwrap();

        // Assert
        assert_eq!(s.play_range_keys, PlayRangeKeys::Brackets, "{key}={raw}");
        assert!(!s.media_open_autoplay && !s.play_ranges_only && !s.collection_play_ranges_only);
        assert_eq!(warnings.len(), 1, "{key}={raw}: {warnings:?}");
        assert!(warnings[0].contains(key));
        assert!(!warnings[0].contains("Brackets") && !warnings[0].contains("not json"), "value leaked");
    }
}

#[test]
fn t_settings_ranges_04_invalid_patch_is_rejected_and_stores_nothing() {
    for (field, bad) in [
        ("playRangeKeys", json!("Brackets")),
        ("playRangeKeys", json!("PREMIERE")),
        ("playRangeKeys", json!(null)),
        ("playRangeKeys", json!(1)),
        ("mediaOpenAutoplay", json!("true")),
        ("mediaOpenAutoplay", json!(1)),
        ("playRangesOnly", json!(null)),
        ("collectionPlayRangesOnly", json!("yes")),
    ] {
        // Arrange
        let mut db = migrated_db();
        let before: i64 = db.conn.query_row("SELECT count(*) FROM settings", [], |r| r.get(0)).unwrap();

        // Act: 유효한 값과 한 패치에 섞어도 전부 거부
        let mut p = patch(json!({"playRangesOnly": true}));
        p.insert(field.to_owned(), bad.clone());
        let err = settings::update(&mut db.conn, &p).unwrap_err();

        // Assert
        let rejected = matches!(&err, SettingsError::Invalid(f) if f == field);
        assert!(rejected, "{field}={bad}: {err:?}");
        let after: i64 = db.conn.query_row("SELECT count(*) FROM settings", [], |r| r.get(0)).unwrap();
        assert_eq!(before, after, "{field}={bad}");
        assert!(!settings::load(&db.conn).unwrap().0.play_ranges_only);
    }
}
