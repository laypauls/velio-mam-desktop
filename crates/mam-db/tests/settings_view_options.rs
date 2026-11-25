//! 보기·동작 설정 `titleEditEnabled` / `sortFieldsEnabled` / `doubleClickAction` 통합 테스트 (T-vopt-01 ~ T-vopt-07).
//!
//! 분류: Happy (01 기본값, 02 왕복) / Validation (03 잘못된 패치, 04 모두 꺼짐 거부) /
//! Boundary (05 하나만 켜짐) / Business rule (06 손상 저장값 대체, 07 한 트랜잭션)

mod common;

use common::{FixedClock, T0, TestDb, migrated_db, shipped_migrations};
use mam_db::{migrate, migrate_with, open, settings};
use tempfile::TempDir;
use mam_db::{
    CapturedAtFallback, DoubleClickAction, GalleryPaging, IndexRailMode, Settings, SettingsError,
    SortFieldsEnabled,
};
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

fn sorts(c: bool, m: bool, n: bool, s: bool, d: bool, r: bool) -> Value {
    json!({"capturedAt": c, "modifiedAt": m, "name": n, "size": s, "duration": d, "rating": r})
}

#[test]
fn t_vopt_01_defaults_when_keys_absent() {
    let db = migrated_db();

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(!s.title_edit_enabled);
    assert_eq!(s.sort_fields_enabled, SortFieldsEnabled::default());
    assert_eq!(s.double_click_action, DoubleClickAction::Preview);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["titleEditEnabled"], json!(false));
    assert_eq!(v["sortFieldsEnabled"], sorts(true, true, true, true, true, true));
    assert_eq!(v["doubleClickAction"], json!("preview"));
    assert!(warnings.is_empty());
}

#[test]
fn t_vopt_02_values_round_trip() {
    let mut db = migrated_db();

    let after = settings::update(
        &mut db.conn,
        &patch(json!({
            "titleEditEnabled": true,
            "sortFieldsEnabled": sorts(true, false, true, false, true, true),
            "doubleClickAction": "openSystem"
        })),
    )
    .unwrap();
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(after, loaded);
    assert!(loaded.title_edit_enabled);
    assert!(!loaded.sort_fields_enabled.size && loaded.sort_fields_enabled.name);
    assert_eq!(loaded.double_click_action, DoubleClickAction::OpenSystem);
    assert_eq!(
        serde_json::to_value(&loaded).unwrap()["doubleClickAction"],
        json!("openSystem")
    );
    assert!(warnings.is_empty());
}

#[test]
fn t_vopt_03_invalid_patch_values_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    let cases = [
        ("titleEditEnabled", json!("yes")),
        ("titleEditEnabled", json!(1)),
        ("doubleClickAction", json!("open")),
        ("doubleClickAction", json!(null)),
        ("doubleClickAction", json!(true)),
        ("sortFieldsEnabled", json!(true)),
        ("sortFieldsEnabled", json!({"capturedAt": true})), // 키 부족
        (
            "sortFieldsEnabled",
            json!({"capturedAt": true, "modifiedAt": true, "name": true, "size": true, "duration": true, "relevance": true}),
        ), // 모르는 키
        (
            "sortFieldsEnabled",
            json!({"capturedAt": "y", "modifiedAt": true, "name": true, "size": true, "duration": true}),
        ), // 불리언 아님
    ];
    for (field, body) in cases {
        let err =
            settings::update(&mut db.conn, &patch(json!({ field: body.clone() }))).unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == field),
            "{field}={body}: {err:?}"
        );
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_vopt_04_all_sort_fields_off_is_rejected() {
    let mut db = migrated_db();

    let err = settings::update(
        &mut db.conn,
        &patch(json!({"sortFieldsEnabled": sorts(false, false, false, false, false, false)})),
    )
    .unwrap_err();

    assert!(matches!(&err, SettingsError::Invalid(f) if f == "sortFieldsEnabled"));
}

#[test]
fn t_vopt_05_a_single_enabled_sort_field_is_accepted() {
    let mut db = migrated_db();

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"sortFieldsEnabled": sorts(false, false, false, false, true, false)})),
    )
    .unwrap();

    assert!(s.sort_fields_enabled.duration);
    assert!(!s.sort_fields_enabled.captured_at);
}

#[test]
fn t_vopt_06_corrupt_stored_values_fall_back_to_defaults_with_warnings() {
    let db = migrated_db();
    put_raw(&db.conn, "title_edit_enabled", "\"nope\"");
    put_raw(
        &db.conn,
        "sort_fields_enabled",
        &sorts(false, false, false, false, false, false).to_string(),
    );
    put_raw(&db.conn, "double_click_action", "\"launch\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(!s.title_edit_enabled);
    assert_eq!(s.sort_fields_enabled, SortFieldsEnabled::default());
    assert_eq!(s.double_click_action, DoubleClickAction::Preview);
    assert_eq!(warnings.len(), 3);
    assert!(
        warnings
            .iter()
            .all(|w| !w.contains("nope") && !w.contains("launch"))
    );
}

#[test]
fn t_vopt_07_one_bad_field_in_a_patch_rolls_back_the_valid_ones() {
    let mut db = migrated_db();

    let err = settings::update(
        &mut db.conn,
        &patch(json!({"titleEditEnabled": true, "doubleClickAction": "bogus"})),
    )
    .unwrap_err();

    assert!(matches!(err, SettingsError::Invalid(_)));
    let (s, _) = settings::load(&db.conn).unwrap();
    assert!(!s.title_edit_enabled);
}

// ---------------------------------------------------------------- 유사 미디어 제한 (T-simset-*)
// Categories: Happy (기본값·왕복) / Boundary (범위 경계) / Validation (범위 밖·타입) / External (손상 저장값)

#[test]
fn t_simset_01_defaults_and_contract_names() {
    let s = Settings::default();
    assert_eq!(s.similar_max_results, 24);
    assert_eq!(s.similar_min_similarity, 0.75);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["similarMaxResults"], json!(24));
    assert_eq!(v["similarMinSimilarity"], json!(0.75));
}

#[test]
fn t_simset_02_round_trip_and_boundaries() {
    let mut db = migrated_db();
    for (max, min) in [(6, 0.0), (200, 0.99), (24, 0.8123)] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"similarMaxResults": max, "similarMinSimilarity": min})),
        )
        .unwrap();
        assert_eq!((s.similar_max_results, s.similar_min_similarity), (max, min));
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_simset_03_out_of_range_and_wrong_types_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    for (field, bad) in [
        ("similarMaxResults", json!(5)),
        ("similarMaxResults", json!(201)),
        ("similarMaxResults", json!(-1)),
        ("similarMaxResults", json!(24.5)),
        ("similarMaxResults", json!("24")),
        ("similarMinSimilarity", json!(-0.01)),
        ("similarMinSimilarity", json!(0.991)),
        ("similarMinSimilarity", json!(1)),
        ("similarMinSimilarity", json!("0.5")),
        ("similarMinSimilarity", Value::Null),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({ field: bad.clone() }))).unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(f) if f == field), "{field}={bad}");
    }
    assert_eq!(settings::load(&db.conn).unwrap().0, Settings::default());
}

#[test]
fn t_simset_04_corrupt_stored_values_fall_back_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "similar_max_results", "9999");
    put_raw(&db.conn, "similar_min_similarity", "\"NaN\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.similar_max_results, 24);
    assert_eq!(s.similar_min_similarity, 0.75);
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

// ---------------------------------------------------------------- 갤러리 탐색 (T-gpage-*)
// Categories: Happy (기본값·왕복) / Boundary (허용 크기 전부) / Validation (범위 밖·타입) / External (손상 저장값)

#[test]
fn t_gpage_01_defaults_and_contract_names() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(s.gallery_paging, GalleryPaging::Infinite);
    assert_eq!(s.gallery_page_size, 100);
    assert_eq!(s.index_rail, IndexRailMode::Auto);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["galleryPaging"], json!("infinite"));
    assert_eq!(v["galleryPageSize"], json!(100));
    assert_eq!(v["indexRail"], json!("auto"));
    assert!(warnings.is_empty());
    assert_eq!(Settings::default(), s);
}

#[test]
fn t_gpage_02_round_trip_all_allowed_values() {
    let mut db = migrated_db();
    for (paging, size, rail) in [
        ("paged", 50, "always"),
        ("infinite", 200, "off"),
        ("paged", 100, "auto"),
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"galleryPaging": paging, "galleryPageSize": size, "indexRail": rail})),
        )
        .unwrap();
        let (loaded, warnings) = settings::load(&db.conn).unwrap();
        assert_eq!(s, loaded);
        let v = serde_json::to_value(&loaded).unwrap();
        assert_eq!(
            (&v["galleryPaging"], &v["galleryPageSize"], &v["indexRail"]),
            (&json!(paging), &json!(size), &json!(rail))
        );
        assert!(warnings.is_empty());
    }
}

#[test]
fn t_gpage_03_out_of_range_and_wrong_types_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    for (field, bad) in [
        ("galleryPaging", json!("scroll")),
        ("galleryPaging", json!(true)),
        ("galleryPaging", Value::Null),
        ("galleryPageSize", json!(0)),
        ("galleryPageSize", json!(49)),
        ("galleryPageSize", json!(75)),
        ("galleryPageSize", json!(201)),
        ("galleryPageSize", json!(500)),
        ("galleryPageSize", json!(-100)),
        ("galleryPageSize", json!(100.5)),
        ("galleryPageSize", json!("100")),
        ("indexRail", json!("sometimes")),
        ("indexRail", json!("Auto")),
        ("indexRail", json!(1)),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({ field: bad.clone() }))).unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(f) if f == field), "{field}={bad}");
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_gpage_04_corrupt_stored_values_fall_back_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "gallery_paging", "\"endless\"");
    put_raw(&db.conn, "gallery_page_size", "75");
    put_raw(&db.conn, "index_rail", "not json");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.gallery_paging, GalleryPaging::Infinite);
    assert_eq!(s.gallery_page_size, 100);
    assert_eq!(s.index_rail, IndexRailMode::Auto);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("endless")));
}

// ---------------------------------------------------------------- 촬영일 대체값 (T-capfb-*)
// Categories: Happy (기본값·왕복) / Validation (잘못된 값·타입) / External (손상 저장값)

#[test]
fn t_capfb_01_default_is_modified_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(s.captured_at_fallback, CapturedAtFallback::Modified);
    assert_eq!(
        serde_json::to_value(&s).unwrap()["capturedAtFallback"],
        json!("modified")
    );
    assert!(warnings.is_empty());
}

#[test]
fn t_capfb_02_both_values_round_trip() {
    let mut db = migrated_db();
    for (raw, expected) in [
        ("none", CapturedAtFallback::None),
        ("modified", CapturedAtFallback::Modified),
    ] {
        let s = settings::update(&mut db.conn, &patch(json!({"capturedAtFallback": raw}))).unwrap();
        let (loaded, warnings) = settings::load(&db.conn).unwrap();
        assert_eq!(s, loaded);
        assert_eq!(loaded.captured_at_fallback, expected);
        assert_eq!(
            serde_json::to_value(&loaded).unwrap()["capturedAtFallback"],
            json!(raw)
        );
        assert!(warnings.is_empty());
    }
}

#[test]
fn t_capfb_03_invalid_strings_and_wrong_types_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    for bad in [
        json!(""),
        json!("created"),
        json!("None"),
        json!("Modified"),
        json!(true),
        json!(1),
        Value::Null,
        json!(["none"]),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({"capturedAtFallback": bad.clone()})))
            .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "capturedAtFallback"),
            "{bad}: {err:?}"
        );
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_capfb_04_corrupt_stored_value_falls_back_to_modified_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "captured_at_fallback", "\"created\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.captured_at_fallback, CapturedAtFallback::Modified);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("created")));
}

// ---------------------------------------------------------------- 별점 설정 (T-rating-set-*)
// Categories: Happy (기본값·왕복) / Boundary (rating 키 누락 -> true) / Validation (잘못된 값) / External (손상 저장값)

#[test]
fn t_rating_set_01_sidebar_rating_menu_defaults_to_true_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert!(s.sidebar_rating_menu);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["sidebarRatingMenu"], json!(true));
    assert_eq!(v["sortFieldsEnabled"]["rating"], json!(true));
    assert!(warnings.is_empty());
}

#[test]
fn t_rating_set_02_sidebar_rating_menu_round_trips() {
    let mut db = migrated_db();
    for flag in [false, true] {
        let s = settings::update(&mut db.conn, &patch(json!({"sidebarRatingMenu": flag}))).unwrap();
        assert_eq!(s.sidebar_rating_menu, flag);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_rating_set_03_non_boolean_sidebar_rating_menu_is_rejected_and_changes_nothing() {
    let mut db = migrated_db();
    for bad in [json!("true"), json!(1), json!(0), Value::Null, json!([true])] {
        let err =
            settings::update(&mut db.conn, &patch(json!({"sidebarRatingMenu": bad.clone()}))).unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "sidebarRatingMenu"), "{bad}");
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_rating_set_04_corrupt_stored_sidebar_rating_menu_falls_back_to_true_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "sidebar_rating_menu", "\"off\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(s.sidebar_rating_menu);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("off\"")));
}

// ---------------------------------------------------------------- 동영상 내장 플레이어 설정 (T-vlcset-*, 구 libVLC 설정 이름)
// Categories: Happy (기본값·왕복) / Validation (잘못된 값) / External (손상 저장값·모르는 키)

#[test]
fn t_vlcset_01_video_player_builtin_defaults_to_true_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert!(s.video_player_builtin);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["videoPlayerBuiltIn"], json!(true));
    assert!(warnings.is_empty());
}

#[test]
fn t_vlcset_02_video_player_builtin_round_trips() {
    let mut db = migrated_db();
    for flag in [false, true] {
        let s = settings::update(&mut db.conn, &patch(json!({"videoPlayerBuiltIn": flag}))).unwrap();
        assert_eq!(s.video_player_builtin, flag);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_vlcset_03_non_boolean_video_player_builtin_is_rejected_and_changes_nothing() {
    let mut db = migrated_db();
    for bad in [json!("true"), json!(1), json!(0), Value::Null, json!([true])] {
        let err =
            settings::update(&mut db.conn, &patch(json!({"videoPlayerBuiltIn": bad.clone()}))).unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "videoPlayerBuiltIn"), "{bad}");
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_vlcset_04_corrupt_stored_video_player_builtin_falls_back_to_true_and_unknown_key_is_ignored() {
    let db = migrated_db();
    put_raw(&db.conn, "video_player_builtin", "\"off\"");
    put_raw(&db.conn, "some_future_setting", "true");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(s.video_player_builtin);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("off\"")));
}

// V27 (video_player_vlc -> video_player_builtin). Categories: Business rule (옛 값 이전·옛 행 보존) / Boundary (옛 행 없음) / Validation (옛 필드 거부)

/// V26 까지만 적용된 DB — "V26 시점 앱"에서 V27 로 올라오는 시나리오
fn v26_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto26: Vec<_> = shipped_migrations().into_iter().filter(|m| m.version() <= 26).collect();
    migrate_with(&mut conn, &upto26, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb { dir, db_path, backup_dir, conn }
}

fn raw_value(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).ok()
}

#[test]
fn t_vlcset_05_old_false_row_is_carried_to_video_player_builtin_by_v27() {
    // Arrange: V26 시점 DB 에 옛 키 false
    let mut db = v26_db();
    put_raw(&db.conn, "video_player_vlc", "false");
    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let (s, warnings) = settings::load(&db.conn).unwrap();
    // Assert
    assert_eq!(report.applied, vec![27]);
    assert!(!s.video_player_builtin);
    assert_eq!(serde_json::to_value(&s).unwrap()["videoPlayerBuiltIn"], json!(false));
    assert!(warnings.is_empty());
    assert_eq!(raw_value(&db.conn, "video_player_builtin").as_deref(), Some("false"));
}

#[test]
fn t_vlcset_06_without_old_row_v27_adds_nothing_and_default_is_true() {
    // Arrange: 옛 행 없는 V26 DB
    let mut db = v26_db();
    // Act
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let (s, warnings) = settings::load(&db.conn).unwrap();
    // Assert
    assert!(s.video_player_builtin);
    assert!(warnings.is_empty());
    let n: i64 = db.conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_vlcset_07_old_field_video_player_vlc_patch_is_rejected_and_changes_nothing() {
    // Arrange
    let mut db = migrated_db();
    // Act
    let err = settings::update(&mut db.conn, &patch(json!({"videoPlayerVlc": false}))).unwrap_err();
    // Assert: 모르는 필드 -> Unknown, 아무 행도 쓰지 않는다
    assert!(matches!(&err, SettingsError::Unknown(f) if f == "videoPlayerVlc"), "{err:?}");
    let n: i64 = db.conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    assert!(settings::load(&db.conn).unwrap().0.video_player_builtin);
}

#[test]
fn t_vlcset_08_v27_keeps_the_old_row_and_does_not_overwrite_an_existing_new_row() {
    // Arrange: 옛 키 false, 새 키가 이미 true 인 V26 DB (재실행·수동 편집 방어)
    let mut db = v26_db();
    put_raw(&db.conn, "video_player_vlc", "false");
    put_raw(&db.conn, "video_player_builtin", "true");
    // Act
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    // Assert: 옛 행 보존(구 앱 롤백용), 새 행은 INSERT OR IGNORE 로 덮어쓰지 않음
    assert_eq!(raw_value(&db.conn, "video_player_vlc").as_deref(), Some("false"));
    assert_eq!(raw_value(&db.conn, "video_player_builtin").as_deref(), Some("true"));
    assert!(settings::load(&db.conn).unwrap().0.video_player_builtin);
}

// ---------------------------------------------------------------- 숨기기 단축키 (T-hidekey-set-*)
// Categories: Happy (기본값·왕복·null) / Validation (형식 위반) / External (손상 저장값)

#[test]
fn t_hidekey_set_01_defaults_to_escape_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(s.hide_shortcut.as_deref(), Some("Escape"));
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["hideShortcut"], json!("Escape"));
    assert!(warnings.is_empty());
}

#[test]
fn t_hidekey_set_02_round_trips_including_null() {
    let mut db = migrated_db();
    let cases = [
        json!("Ctrl+Shift+H"),
        json!("F12"),
        json!("Alt+X"),
        json!("Escape"),
        Value::Null,
        json!("a".repeat(20)),
    ];
    for want in cases {
        let s = settings::update(&mut db.conn, &patch(json!({"hideShortcut": want.clone()})))
            .unwrap();
        assert_eq!(serde_json::to_value(&s).unwrap()["hideShortcut"], want);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_hidekey_set_03_invalid_values_are_rejected_and_change_nothing() {
    let mut db = migrated_db();
    let bad_values = [
        json!(""),
        json!("Shift+Ctrl+A"),
        json!("Ctrl+"),
        json!("Ctrl+Shift+"),
        json!("Ctrl + A"),
        json!("Space Bar"),
        json!("A+B"),
        json!("a".repeat(21)),
        json!(1),
        json!(true),
        json!(["Escape"]),
    ];
    for bad in bad_values {
        let err = settings::update(&mut db.conn, &patch(json!({"hideShortcut": bad.clone()})))
            .unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "hideShortcut"), "{bad}");
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn t_hidekey_set_04_corrupt_stored_value_falls_back_to_escape_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "hide_shortcut", "\"Shift+Ctrl+A\"");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.hide_shortcut.as_deref(), Some("Escape"));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings.iter().all(|w| !w.contains("Shift+Ctrl")));
}

#[test]
fn t_rating_set_05_sort_fields_without_rating_key_default_rating_to_true() {
    // Arrange: 별점 도입 전 앱이 저장한 5키 값
    let db = migrated_db();
    put_raw(
        &db.conn,
        "sort_fields_enabled",
        r#"{"capturedAt": true, "modifiedAt": false, "name": true, "size": false, "duration": true}"#,
    );

    // Act
    let (s, warnings) = settings::load(&db.conn).unwrap();

    // Assert: 값은 유지되고 rating 만 기본 true, 경고 없음
    assert!(s.sort_fields_enabled.rating);
    assert!(!s.sort_fields_enabled.modified_at && s.sort_fields_enabled.name);
    assert!(warnings.is_empty());
}

#[test]
fn t_rating_set_06_sort_fields_rating_round_trips_and_validation_still_holds() {
    let mut db = migrated_db();

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"sortFieldsEnabled": sorts(true, true, true, true, true, false)})),
    )
    .unwrap();
    assert!(!s.sort_fields_enabled.rating);
    // 별점만 켜진 경우는 "하나 이상 켜짐" 이므로 허용
    let only = settings::update(
        &mut db.conn,
        &patch(json!({"sortFieldsEnabled": sorts(false, false, false, false, false, true)})),
    )
    .unwrap();
    assert!(only.sort_fields_enabled.rating && !only.sort_fields_enabled.name);
    // 별점 값이 불리언이 아니거나 모르는 키가 하나 더 있으면 거부
    for bad in [
        json!({"capturedAt": true, "modifiedAt": true, "name": true, "size": true, "duration": true, "rating": "y"}),
        json!({"capturedAt": true, "modifiedAt": true, "name": true, "size": true, "duration": true, "rating": true, "extra": true}),
        json!({"modifiedAt": true, "name": true, "size": true, "duration": true, "rating": true}),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({"sortFieldsEnabled": bad.clone()})))
            .unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "sortFieldsEnabled"), "{bad}");
    }
}

// ---------------------------------------------------------------- 연속 재생 이미지 표시 시간 (T-set-ss-*)
// Categories: Happy (기본값·경계 왕복) / Validation (범위·타입 위반) / External (손상 저장값)

#[test]
fn t_set_ss_01_default_is_5_bounds_round_trip_and_invalid_values_are_rejected() {
    // Arrange
    let mut db = migrated_db();
    let (s, _) = settings::load(&db.conn).unwrap();
    assert_eq!(s.slideshow_image_seconds, 5);
    assert_eq!(serde_json::to_value(&s).unwrap()["slideshowImageSeconds"], json!(5));

    // Act + Assert: 경계값 1·60 은 통과
    for ok in [1, 60] {
        let s = settings::update(&mut db.conn, &patch(json!({"slideshowImageSeconds": ok})))
            .unwrap();
        assert_eq!(s.slideshow_image_seconds, ok);
    }
    // 범위 밖·잘못된 타입은 거부하고 마지막 값(60)을 유지
    for bad in [json!(0), json!(61), json!(-1), json!(2.5), json!("5"), Value::Null] {
        let err = settings::update(&mut db.conn, &patch(json!({"slideshowImageSeconds": bad.clone()})))
            .unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "slideshowImageSeconds"), "{bad}");
    }
    assert_eq!(settings::load(&db.conn).unwrap().0.slideshow_image_seconds, 60);
}

#[test]
fn t_set_ss_02_corrupt_stored_value_falls_back_to_5_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "slideshow_image_seconds", "61");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.slideshow_image_seconds, 5);
    assert!(warnings.iter().any(|w| w.contains("slideshow_image_seconds")), "{warnings:?}");
}
