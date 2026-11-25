//! `playerVolume`(T-set-vol-*) 설정 통합 테스트 (실제 SQLite).
//!
//! 분류: Happy (기본값·왕복) / Boundary (0·100 경계, clamp) / Validation (비객체 거부) /
//! Business rule (필드 단위 정규화, 음소거와 level 분리, 손상 저장값 읽기 복구).

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

fn vol_json(s: &mam_db::Settings) -> Value {
    serde_json::to_value(s).unwrap()["playerVolume"].clone()
}

#[test]
fn t_set_vol_01_defaults_to_100_unmuted() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert_eq!(vol_json(&s), json!({"level": 100, "muted": false}));
    assert!(warnings.is_empty());
}

#[test]
fn t_set_vol_02_saves_and_reloads_level_and_muted_separately() {
    let mut db = migrated_db();
    let want = json!({"level": 35, "muted": true});

    let s = settings::update(&mut db.conn, &patch(json!({"playerVolume": want.clone()}))).unwrap();

    assert_eq!(vol_json(&s), want);
    assert_eq!(vol_json(&settings::load(&db.conn).unwrap().0), want);
    // 음소거 해제 — level 은 그대로 유지된다
    let s = settings::update(
        &mut db.conn,
        &patch(json!({"playerVolume": {"level": 35, "muted": false}})),
    )
    .unwrap();
    assert_eq!(vol_json(&s), json!({"level": 35, "muted": false}));
}

#[test]
fn t_set_vol_03_level_clamps_to_0_100_and_rounds() {
    let mut db = migrated_db();
    for (input, want) in [
        (json!(-5), 0),
        (json!(0), 0),
        (json!(100), 100),
        (json!(150), 100),
        (json!(49.6), 50),
        (json!(1_000_000_000_000u64), 100),
    ] {
        let s = settings::update(
            &mut db.conn,
            &patch(json!({"playerVolume": {"level": input.clone()}})),
        )
        .unwrap();
        assert_eq!(vol_json(&s)["level"], json!(want), "{input}");
    }
}

#[test]
fn t_set_vol_04_bad_fields_normalize_to_defaults_without_failing() {
    let mut db = migrated_db();

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"playerVolume": {"level": "loud", "muted": 1, "futureField": 1}})),
    )
    .unwrap();
    assert_eq!(vol_json(&s), json!({"level": 100, "muted": false}));

    // 한 필드만 깨지면 그 필드만 기본값
    let s = settings::update(
        &mut db.conn,
        &patch(json!({"playerVolume": {"level": 20, "muted": "yes"}})),
    )
    .unwrap();
    assert_eq!(vol_json(&s), json!({"level": 20, "muted": false}));

    let s = settings::update(
        &mut db.conn,
        &patch(json!({"playerVolume": {"muted": true}})),
    )
    .unwrap();
    assert_eq!(vol_json(&s), json!({"level": 100, "muted": true}));
}

#[test]
fn t_set_vol_05_non_object_patch_is_invalid_and_changes_nothing() {
    let mut db = migrated_db();
    for bad in [
        json!(null),
        json!(true),
        json!("loud"),
        json!(50),
        json!([1]),
    ] {
        let err = settings::update(&mut db.conn, &patch(json!({"playerVolume": bad.clone()})))
            .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "playerVolume"),
            "{bad}"
        );
    }
    assert_eq!(count_rows(&db.conn), 0);
}

#[test]
fn t_set_vol_06_corrupt_stored_values_recover_on_read() {
    let db = migrated_db();
    put_raw(&db.conn, "player_volume", r#"{"level":999,"muted":"x"}"#);

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert_eq!(vol_json(&s), json!({"level": 100, "muted": false}));
    assert!(warnings.is_empty(), "{warnings:?}");

    db.conn
        .execute(
            "UPDATE settings SET value = '\"junk\"' WHERE key = 'player_volume'",
            [],
        )
        .unwrap();
    let (s, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(vol_json(&s), json!({"level": 100, "muted": false}));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}
