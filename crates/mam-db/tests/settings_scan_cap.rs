//! 설정 `maxConcurrentScans` (스캔 전체 동시성 상한, 결정 E) 통합 테스트 (T-lane-06s ~ T-lane-07s).
//!
//! 분류: Happy (기본값 3·경계 1/8 왕복) / Validation (범위·타입 위반 거부, 상태 불변) / External (손상 저장값은 기본값 + 경고)

mod common;

use common::migrated_db;
use mam_db::settings;
use mam_db::SettingsError;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn put_raw(conn: &Connection, key: &str, raw: &str) {
    conn.execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [key, raw]).unwrap();
}

#[test]
fn t_lane_06s_default_is_3_bounds_round_trip_and_invalid_values_are_rejected() {
    // Arrange
    let mut db = migrated_db();
    let (s, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(s.max_concurrent_scans, 3);
    assert!(warnings.is_empty());
    assert_eq!(serde_json::to_value(&s).unwrap()["maxConcurrentScans"], json!(3));

    // Act + Assert: 경계 1·8 은 통과
    for ok in [1, 8, 4] {
        let s = settings::update(&mut db.conn, &patch(json!({"maxConcurrentScans": ok}))).unwrap();
        assert_eq!(s.max_concurrent_scans, ok);
    }
    // 범위 밖·잘못된 타입은 거부하고 마지막 값(4)을 유지한다
    for bad in [json!(0), json!(9), json!(-1), json!(2.5), json!("3"), Value::Null, json!(true)] {
        let err = settings::update(&mut db.conn, &patch(json!({"maxConcurrentScans": bad.clone()}))).unwrap_err();
        assert!(matches!(&err, SettingsError::Invalid(f) if f == "maxConcurrentScans"), "{bad}");
    }
    assert_eq!(settings::load(&db.conn).unwrap().0.max_concurrent_scans, 4);
}

#[test]
fn t_lane_07s_an_invalid_value_in_a_patch_changes_nothing_and_a_corrupt_stored_value_falls_back() {
    // Arrange
    let mut db = migrated_db();
    settings::update(&mut db.conn, &patch(json!({"maxConcurrentScans": 2}))).unwrap();

    // Act: 같은 패치의 다른 값이 정상이어도 하나가 잘못되면 전부 거부된다
    let err = settings::update(
        &mut db.conn,
        &patch(json!({"maxConcurrentScans": 99, "rescanIntervalHours": 12})),
    )
    .unwrap_err();

    // Assert
    assert!(matches!(err, SettingsError::Invalid(_)));
    let s = settings::load(&db.conn).unwrap().0;
    assert_eq!((s.max_concurrent_scans, s.rescan_interval_hours), (2, 6));

    // 손상된 저장값(직접 편집·이후 버전)은 기본값으로 읽고 경고한다
    let db2 = migrated_db();
    put_raw(&db2.conn, "max_concurrent_scans", "0");
    let (s, warnings) = settings::load(&db2.conn).unwrap();
    assert_eq!(s.max_concurrent_scans, 3);
    assert!(warnings.iter().any(|w| w.contains("max_concurrent_scans")), "{warnings:?}");
}
