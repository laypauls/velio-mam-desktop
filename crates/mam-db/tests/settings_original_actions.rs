//! 설정 `originalActionsEnabled` (원본 이동·삭제 UI 노출 여부) 통합 테스트 (T-oact-01 ~ T-oact-03).
//!
//! 분류: Happy (01 기본값 false, 02 왕복) / Validation (03 불리언 아닌 값 거부)

mod common;

use common::migrated_db;
use mam_db::SettingsError;
use mam_db::settings;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn t_oact_01_default_is_false() {
    let db = migrated_db();

    let (s, warnings) = settings::load(&db.conn).unwrap();

    assert!(!s.original_actions_enabled);
    assert_eq!(
        serde_json::to_value(&s).unwrap()["originalActionsEnabled"],
        json!(false)
    );
    assert!(warnings.is_empty());
}

#[test]
fn t_oact_02_value_round_trips() {
    let mut db = migrated_db();

    let after = settings::update(
        &mut db.conn,
        &patch(json!({"originalActionsEnabled": true})),
    )
    .unwrap();
    let (loaded, _) = settings::load(&db.conn).unwrap();

    assert_eq!(after, loaded);
    assert!(loaded.original_actions_enabled);
    let back = settings::update(
        &mut db.conn,
        &patch(json!({"originalActionsEnabled": false})),
    )
    .unwrap();
    assert!(!back.original_actions_enabled);
}

#[test]
fn t_oact_03_non_boolean_is_rejected_and_changes_nothing() {
    let mut db = migrated_db();

    for body in [json!("yes"), json!(1), json!(null)] {
        let err = settings::update(
            &mut db.conn,
            &patch(json!({"originalActionsEnabled": body})),
        )
        .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Invalid(f) if f == "originalActionsEnabled"),
            "{body}: {err:?}"
        );
    }
    let (s, _) = settings::load(&db.conn).unwrap();
    assert!(!s.original_actions_enabled);
}
