//! 갤러리 카드 표시 설정 `galleryCardDisplay` 통합 테스트 (T-gcard-01 ~ T-gcard-12).
//!
//! cardSize: 09 구 저장값(필드 없음) 기본 M / 10 다섯 값 왕복 / 11 잘못된 패치 거부 / 12 손상 저장값 기본값.
//!
//! 분류: Happy (01 기본값, 02 왕복) / Validation (03 잘못된 패치, 07 손상 저장값) /
//! Boundary (04 빠진 필드, 05 모르는 필드, 08 enum 전체 값) / Concurrency·Business rule (06 한 트랜잭션).

mod common;

use common::migrated_db;
use mam_db::settings;
use mam_db::{CardCheck, CardDateBy, CardSize, GalleryCardDisplay, Settings, SettingsError};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

const KEY: &str = "gallery_card_display";

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn put_raw(conn: &Connection, raw: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)",
        [KEY, raw],
    )
    .unwrap();
}

fn row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap()
}

fn full(info_name: bool, date_by: &str, check: &str) -> Value {
    json!({
        "info": {"typeIcon": false, "name": info_name, "ext": true, "resolution": false, "size": true, "date": false},
        "dateBy": date_by, "check": check, "cardSize": "l",
        "favorite": false, "rating": false, "duration": true, "quickActions": false
    })
}

#[test]
fn t_gcard_01_default_when_key_absent() {
    let db = migrated_db();

    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(loaded.gallery_card_display, GalleryCardDisplay::default());
    let d = &loaded.gallery_card_display;
    assert!(d.info.type_icon && d.info.name && d.info.ext && d.info.resolution);
    assert!(d.info.size && d.info.date && d.favorite && d.duration && d.quick_actions);
    assert_eq!(
        (d.date_by, d.check),
        (CardDateBy::Captured, CardCheck::Hover)
    );
    assert!(warnings.is_empty());
}

#[test]
fn t_gcard_02_full_object_round_trips_through_reopen() {
    let mut db = migrated_db();
    let body = full(false, "modified", "always");

    let updated =
        settings::update(&mut db.conn, &patch(json!({"galleryCardDisplay": body}))).unwrap();
    drop(db.conn);
    let reopened = mam_db::open(&db.db_path).unwrap();
    let (after, warnings) = settings::load(&reopened).unwrap();

    assert_eq!(after, updated);
    assert!(warnings.is_empty());
    assert_eq!(
        serde_json::to_value(&after.gallery_card_display).unwrap(),
        body
    );
    assert_eq!(after.gallery_card_display.date_by, CardDateBy::Modified);
    assert_eq!(after.gallery_card_display.check, CardCheck::Always);
}

#[test]
fn t_gcard_03_invalid_patch_is_rejected_and_nothing_is_written() {
    let mut db = migrated_db();
    let cases = [
        json!("captured"),
        json!(null),
        json!([]),
        json!({"info": true}),
        json!({"info": {"name": "yes"}}),
        json!({"info": {"size": 1}}),
        json!({"dateBy": "created"}),
        json!({"dateBy": 1}),
        json!({"dateBy": null}),
        json!({"check": "Hover"}),
        json!({"check": false}),
        json!({"favorite": "true"}),
        json!({"duration": null}),
        json!({"quickActions": 0}),
    ];
    for body in cases {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"galleryCardDisplay": body.clone()})),
        )
        .unwrap_err();

        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "galleryCardDisplay"),
            "{body}: {err:?}"
        );
    }
    assert_eq!(row_count(&db.conn), 0, "nothing written");
    assert_eq!(settings::load(&db.conn).unwrap().0, Settings::default());
}

#[test]
fn t_gcard_04_missing_fields_default_per_field() {
    let db = migrated_db();
    put_raw(
        &db.conn,
        r#"{"info":{"name":false},"check":"hidden","favorite":false}"#,
    );

    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    let d = loaded.gallery_card_display;
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!d.info.name && !d.favorite, "present fields are kept");
    assert_eq!(d.check, CardCheck::Hidden);
    assert!(d.info.type_icon && d.info.ext && d.info.resolution && d.info.size && d.info.date);
    assert_eq!(d.date_by, CardDateBy::Captured);
    assert!(d.duration && d.quick_actions, "missing fields use defaults");
}

#[test]
fn t_gcard_05_unknown_fields_are_ignored_on_load_and_update() {
    let mut db = migrated_db();
    put_raw(
        &db.conn,
        r#"{"future":1,"info":{"name":false,"newField":"x"},"dateBy":"modified"}"#,
    );

    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    let updated = settings::update(
        &mut db.conn,
        &patch(json!({"galleryCardDisplay": {"extra": [1], "check": "always"}})),
    )
    .unwrap();

    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(!loaded.gallery_card_display.info.name);
    assert_eq!(loaded.gallery_card_display.date_by, CardDateBy::Modified);
    assert_eq!(updated.gallery_card_display.check, CardCheck::Always);
}

#[test]
fn t_gcard_06_patch_with_another_invalid_setting_writes_nothing() {
    let mut db = migrated_db();
    let body = full(false, "modified", "hidden");

    let err = settings::update(
        &mut db.conn,
        &patch(json!({"galleryCardDisplay": body, "authExemptMinutes": 61})),
    )
    .unwrap_err();
    let ok_first = settings::update(
        &mut db.conn,
        &patch(json!({"galleryCardDisplay": {"check": "always"}, "closeToTray": false})),
    )
    .unwrap();

    assert!(matches!(err, SettingsError::Invalid(f) if f == "authExemptMinutes"));
    assert_eq!(ok_first.gallery_card_display.check, CardCheck::Always);
    assert_eq!(row_count(&db.conn), 2, "only the valid patch is stored");
    assert_eq!(
        ok_first.gallery_card_display.date_by,
        CardDateBy::Captured,
        "rejected patch left no trace"
    );
}

#[test]
fn t_gcard_07_corrupted_stored_value_defaults_with_key_only_warning() {
    for raw in [
        "not json {{secret-token",
        "\"secret-token\"",
        "[1,2]",
        "null",
        r#"{"dateBy":"secret-token"}"#,
        r#"{"info":{"name":"secret-token"}}"#,
    ] {
        let db = migrated_db();
        put_raw(&db.conn, raw);

        let (loaded, warnings) = settings::load(&db.conn).unwrap();

        assert_eq!(loaded, Settings::default(), "{raw}");
        assert_eq!(warnings.len(), 1, "{raw}: {warnings:?}");
        assert!(warnings[0].contains(KEY));
        assert!(!warnings[0].contains("secret-token"), "value never logged");
    }
}

#[test]
fn t_gcard_08_every_enum_value_is_accepted() {
    let mut db = migrated_db();
    for (by, check) in [
        ("captured", "hover"),
        ("modified", "always"),
        ("captured", "hidden"),
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"galleryCardDisplay": full(true, by, check)})),
        )
        .unwrap();

        let v = serde_json::to_value(&s.gallery_card_display).unwrap();
        assert_eq!(
            (v["dateBy"].as_str(), v["check"].as_str()),
            (Some(by), Some(check))
        );
    }
}

#[test]
fn t_gcard_09_missing_card_size_defaults_to_m() {
    let db = migrated_db();
    // cardSize 도입 전 앱이 저장한 값 — 그대로 유효해야 한다(롤백·전방 호환).
    put_raw(&db.conn, r#"{"dateBy":"modified","check":"always"}"#);

    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(loaded.gallery_card_display.card_size, CardSize::M);
    assert_eq!(loaded.gallery_card_display.check, CardCheck::Always);
    assert_eq!(GalleryCardDisplay::default().card_size, CardSize::M);
}

#[test]
fn t_gcard_10_every_card_size_round_trips_through_save_and_reopen() {
    let mut db = migrated_db();
    for (raw, expected) in [
        ("xs", CardSize::Xs),
        ("s", CardSize::S),
        ("m", CardSize::M),
        ("l", CardSize::L),
        ("xl", CardSize::Xl),
    ] {
        let updated = settings::update(
            &mut db.conn,
            &patch(json!({"galleryCardDisplay": {"cardSize": raw}})),
        )
        .unwrap();
        let v = serde_json::to_value(&updated.gallery_card_display).unwrap();
        let (loaded, warnings) = settings::load(&db.conn).unwrap();

        assert_eq!(v["cardSize"], json!(raw));
        assert_eq!(updated.gallery_card_display.card_size, expected);
        assert_eq!(
            GalleryCardDisplay::parse(&v),
            Some(updated.gallery_card_display.clone())
        );
        assert_eq!(loaded, updated, "{raw}");
        assert!(warnings.is_empty());
    }
    drop(db.conn);
    let reopened = mam_db::open(&db.db_path).unwrap();
    let (after, _) = settings::load(&reopened).unwrap();
    assert_eq!(after.gallery_card_display.card_size, CardSize::Xl);
}

#[test]
fn t_gcard_11_invalid_card_size_is_rejected_and_nothing_is_written() {
    let mut db = migrated_db();
    for bad in [
        json!("xxl"),
        json!("M"),
        json!(""),
        json!(3),
        json!(null),
        json!(true),
    ] {
        let body = json!({"cardSize": bad.clone()});
        assert!(GalleryCardDisplay::parse(&body).is_none(), "{bad}");

        let err = settings::update(&mut db.conn, &patch(json!({"galleryCardDisplay": body})))
            .unwrap_err();

        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "galleryCardDisplay"),
            "{bad}: {err:?}"
        );
    }
    assert_eq!(row_count(&db.conn), 0, "nothing written");
}

#[test]
fn t_gcard_12_corrupted_card_size_in_store_defaults_with_key_only_warning() {
    let db = migrated_db();
    put_raw(&db.conn, r#"{"cardSize":"secret-token"}"#);

    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(loaded, Settings::default());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains(KEY) && !warnings[0].contains("secret-token"));
}

// ---------------------------------------------------------------- 카드 별점 표시 (T-rating-gcard-*)
// Categories: Happy (기본 true·왕복) / Boundary (키 누락 -> 기본값) / Validation (불리언 아님)

#[test]
fn t_rating_gcard_01_missing_rating_key_defaults_to_true_while_other_fields_are_kept() {
    // Arrange: 별점 도입 전 앱이 저장한 값 (rating 없음)
    let db = migrated_db();
    put_raw(&db.conn, r#"{"favorite": false}"#);

    // Act
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert!(loaded.gallery_card_display.rating);
    assert!(!loaded.gallery_card_display.favorite);
    assert!(warnings.is_empty());
    assert_eq!(
        serde_json::to_value(&loaded.gallery_card_display).unwrap()["rating"],
        json!(true)
    );
}

#[test]
fn t_rating_gcard_02_false_round_trips_and_non_boolean_is_rejected() {
    let mut db = migrated_db();

    let after =
        settings::update(&mut db.conn, &patch(json!({"galleryCardDisplay": {"rating": false}})))
            .unwrap();
    assert!(!after.gallery_card_display.rating);
    assert_eq!(settings::load(&db.conn).unwrap().0, after);

    for bad in [json!("yes"), json!(0), json!(null)] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"galleryCardDisplay": {"rating": bad.clone()}})),
        )
        .unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "galleryCardDisplay"), "{bad}");
    }
}
