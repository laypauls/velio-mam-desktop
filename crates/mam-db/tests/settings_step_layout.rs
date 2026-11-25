//! `playerPlayingStepSec`(T-set-step-*)·`layout`(T-set-layout-*) 설정 통합 테스트 (실제 SQLite).
//!
//! 분류: Happy (기본값·왕복) / Boundary (1·60 허용, 0·61 거부, 폭 clamp) / Validation (타입 오류·비객체 거부) /
//! Business rule (잘못된 필드 정규화, 손상 저장값 읽기 복구).

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

fn count_rows(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap()
}

fn layout_json(s: &mam_db::Settings) -> Value {
    serde_json::to_value(s).unwrap()["layout"].clone()
}

#[test]
fn t_set_step_01_defaults_to_5_with_contract_name() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(s.player_playing_step_sec, 5);
    assert_eq!(serde_json::to_value(&s).unwrap()["playerPlayingStepSec"], json!(5));
    assert!(warnings.is_empty());
}

#[test]
fn t_set_step_02_boundaries_1_and_60_round_trip_across_reload() {
    let mut db = migrated_db();
    for ok in [1, 15, 60] {
        let s = settings::update(&mut db.conn, &patch(json!({"playerPlayingStepSec": ok}))).unwrap();
        assert_eq!(s.player_playing_step_sec, ok);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
}

#[test]
fn t_set_step_03_out_of_range_or_wrong_type_is_invalid_and_changes_nothing() {
    let mut db = migrated_db();
    for bad in [json!(0), json!(61), json!(-1), json!(2.5), json!("5"), json!(null), json!(true)] {
        let err = settings::update(&mut db.conn, &patch(json!({"playerPlayingStepSec": bad.clone()})))
            .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "playerPlayingStepSec"),
            "{bad}"
        );
    }
    assert_eq!(count_rows(&db.conn), 0);
}

#[test]
fn t_set_step_04_corrupt_stored_value_falls_back_to_5_with_a_warning() {
    let db = migrated_db();
    put_raw(&db.conn, "player_playing_step_sec", "61");

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(s.player_playing_step_sec, 5);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn t_set_layout_01_defaults_match_contract_shape() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(
        layout_json(&s),
        json!({"detailOpen": true, "sidebarOpen": true, "viewerWidth": 520, "detailTab": "info", "filmstripHeight": 104})
    );
    assert!(warnings.is_empty());
}

#[test]
fn t_set_layout_02_saves_and_reloads_every_field() {
    let mut db = migrated_db();
    let want = json!({"detailOpen": false, "sidebarOpen": false, "viewerWidth": 777, "detailTab": "similar", "filmstripHeight": 120});

    let s = settings::update(&mut db.conn, &patch(json!({"layout": want.clone()}))).unwrap();

    assert_eq!(layout_json(&s), want);
    assert_eq!(layout_json(&settings::load(&db.conn).unwrap().0), want);
    for tab in ["info", "ai", "similar"] {
        let s = settings::update(&mut db.conn, &patch(json!({"layout": {"detailTab": tab}}))).unwrap();
        assert_eq!(layout_json(&s)["detailTab"], json!(tab));
    }
}

#[test]
fn t_set_layout_03_missing_fields_default_per_field_and_unknown_fields_are_ignored() {
    let mut db = migrated_db();

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"layout": {"detailOpen": false, "futureField": 1}})),
    )
    .unwrap();

    assert_eq!(
        layout_json(&s),
        json!({"detailOpen": false, "sidebarOpen": true, "viewerWidth": 520, "detailTab": "info", "filmstripHeight": 104})
    );
}

#[test]
fn t_set_layout_04_bad_fields_normalize_to_defaults_without_failing() {
    let mut db = migrated_db();

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"layout": {
            "detailOpen": "yes", "sidebarOpen": 0, "viewerWidth": "wide", "detailTab": "nope", "filmstripHeight": "tall"
        }})),
    )
    .unwrap();

    assert_eq!(
        layout_json(&s),
        json!({"detailOpen": true, "sidebarOpen": true, "viewerWidth": 520, "detailTab": "info", "filmstripHeight": 104})
    );
}

#[test]
fn t_set_layout_05_viewer_width_clamps_to_min_320_and_rounds() {
    let mut db = migrated_db();
    for (input, want) in [
        (json!(319), 320),
        (json!(0), 320),
        (json!(-50), 320),
        (json!(320), 320),
        (json!(600.6), 601),
        (json!(1_000_000_000_000u64), 8192),
    ] {
        let s = settings::update(&mut db.conn, &patch(json!({"layout": {"viewerWidth": input.clone()}})))
            .unwrap();
        assert_eq!(layout_json(&s)["viewerWidth"], json!(want), "{input}");
    }
}

#[test]
fn t_set_layout_06_non_object_patch_is_invalid_and_changes_nothing() {
    let mut db = migrated_db();
    for bad in [json!(null), json!(true), json!("open"), json!(5), json!([1])] {
        let err = settings::update(&mut db.conn, &patch(json!({"layout": bad.clone()}))).unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "layout"), "{bad}");
    }
    assert_eq!(count_rows(&db.conn), 0);
}

#[test]
fn t_set_layout_07_corrupt_stored_values_recover_on_read() {
    let db = migrated_db();
    // 객체지만 필드가 깨진 값은 필드 단위로 복구, 객체가 아니면 기본값 + 경고
    put_raw(&db.conn, "layout", r#"{"detailOpen":false,"viewerWidth":10,"detailTab":7}"#);

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(
        layout_json(&s),
        json!({"detailOpen": false, "sidebarOpen": true, "viewerWidth": 320, "detailTab": "info", "filmstripHeight": 104})
    );
    assert!(warnings.is_empty(), "{warnings:?}");

    db.conn.execute("UPDATE settings SET value = '\"junk\"' WHERE key = 'layout'", []).unwrap();
    let (s, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(layout_json(&s)["detailOpen"], json!(true));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

#[test]
fn t_set_layout_08_filmstrip_height_defaults_to_104_when_missing_and_round_trips() {
    let mut db = migrated_db();

    // 필드가 없는 기존(구버전) 저장값 -> 기본값, 다른 필드는 그대로
    put_raw(&db.conn, "layout", r#"{"detailOpen":false,"sidebarOpen":false,"viewerWidth":700,"detailTab":"ai"}"#);
    let (s, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(
        layout_json(&s),
        json!({"detailOpen": false, "sidebarOpen": false, "viewerWidth": 700, "detailTab": "ai", "filmstripHeight": 104})
    );
    assert!(warnings.is_empty(), "{warnings:?}");

    // 유효 값 왕복 (경계 72·160 포함) 과 기존 필드 불변
    for ok in [72, 100, 160] {
        let s = settings::update(&mut db.conn, &patch(json!({"layout": {
            "detailOpen": false, "sidebarOpen": false, "viewerWidth": 700, "detailTab": "ai", "filmstripHeight": ok
        }})))
        .unwrap();
        assert_eq!(layout_json(&s)["filmstripHeight"], json!(ok));
        assert_eq!(layout_json(&settings::load(&db.conn).unwrap().0), layout_json(&s));
        assert_eq!(layout_json(&s)["viewerWidth"], json!(700));
        assert_eq!(layout_json(&s)["detailTab"], json!("ai"));
    }
}

#[test]
fn t_set_layout_09_filmstrip_height_clamps_to_72_160_and_rounds() {
    let mut db = migrated_db();
    for (input, want) in [
        (json!(71), 72),
        (json!(0), 72),
        (json!(-5), 72),
        (json!(72), 72),
        (json!(160), 160),
        (json!(161), 160),
        (json!(1_000_000_000_000u64), 160),
        (json!(100.6), 101),
    ] {
        let s = settings::update(&mut db.conn, &patch(json!({"layout": {"filmstripHeight": input.clone()}})))
            .unwrap();
        assert_eq!(layout_json(&s)["filmstripHeight"], json!(want), "{input}");
    }
}

#[test]
fn t_set_layout_10_filmstrip_height_wrong_type_falls_back_to_default() {
    let mut db = migrated_db();
    for bad in [json!("120"), json!(null), json!(true), json!([120]), json!({"h": 1})] {
        let s = settings::update(&mut db.conn, &patch(json!({"layout": {"filmstripHeight": bad.clone()}})))
            .unwrap();
        assert_eq!(layout_json(&s)["filmstripHeight"], json!(104), "{bad}");
    }
}
