//! `resumeEnabled`·`resumeMinDurationSec` 설정 (T-resume-set-*) — 실제 SQLite.
//!
//! 분류: Happy (기본값·왕복) / Boundary (허용 값 0·60·180·300·600) / Validation (그 밖의 값·타입 거부).

mod common;

use common::migrated_db;
use mam_db::SettingsError;
use mam_db::settings;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn t_resume_set_01_defaults_with_contract_names() {
    let (s, warnings) = settings::load(&migrated_db().conn).unwrap();

    assert!(s.resume_enabled);
    assert_eq!(s.resume_min_duration_sec, 180);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["resumeEnabled"], json!(true));
    assert_eq!(v["resumeMinDurationSec"], json!(180));
    assert!(warnings.is_empty());
}

#[test]
fn t_resume_set_02_allowed_values_round_trip() {
    let mut db = migrated_db();
    for ok in [0, 60, 180, 300, 600] {
        let s =
            settings::update(&mut db.conn, &patch(json!({"resumeMinDurationSec": ok}))).unwrap();
        assert_eq!(s.resume_min_duration_sec, ok);
        assert_eq!(settings::load(&db.conn).unwrap().0, s);
    }
    let s = settings::update(&mut db.conn, &patch(json!({"resumeEnabled": false}))).unwrap();
    assert!(!s.resume_enabled);
    assert!(!settings::load(&db.conn).unwrap().0.resume_enabled);
}

#[test]
fn t_resume_set_03_other_values_are_invalid_and_change_nothing() {
    let mut db = migrated_db();
    for bad in [
        json!(1),
        json!(120),
        json!(601),
        json!(-60),
        json!(180.5),
        json!("180"),
        json!(null),
    ] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"resumeMinDurationSec": bad.clone()})),
        )
        .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "resumeMinDurationSec"),
            "{bad}"
        );
    }
    for bad in [json!(1), json!("true"), json!(null)] {
        let err = settings::update(&mut db.conn, &patch(json!({"resumeEnabled": bad.clone()})))
            .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "resumeEnabled"),
            "{bad}"
        );
    }
    let n: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
