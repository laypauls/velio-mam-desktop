//! "묻지 않는 디스크" 영속 상태 `ignored_volumes` 통합 테스트 (T-ignvol-db-01 ~ 08).
//!
//! 분류: Happy (01 왕복, 05 갱신) / Boundary (02 키 없음, 06 없는 키 제거·갱신) / Validation (03 깨진 JSON) /
//! Business rule (04 중복 추가, 07 설정 계층에 비노출) / Concurrency (08 반복 갱신 멱등).

mod common;

use common::migrated_db;
use mam_db::ignored_volumes::{
    IgnoredVolume, add_ignored_volume, load_ignored_volumes, remove_ignored_volume,
    update_ignored_volume,
};
use mam_db::settings;
use serde_json::{Map, json};

fn vol(key: &str, label: Option<&str>, mount: Option<&str>, at: i64) -> IgnoredVolume {
    IgnoredVolume {
        volume_key: key.into(),
        label: label.map(Into::into),
        last_mount_point: mount.map(Into::into),
        ignored_at: at,
    }
}

fn put_raw(conn: &rusqlite::Connection, raw: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('ignored_volumes', ?1)",
        [raw],
    )
    .unwrap();
}

#[test]
fn t_ignvol_db_01_add_then_load_round_trips_with_camel_case_json() {
    // Arrange
    let db = migrated_db();
    let a = vol("vol-a", Some("USB"), Some(r"E:\"), 10);

    // Act
    add_ignored_volume(&db.conn, &a).unwrap();

    // Assert
    assert_eq!(load_ignored_volumes(&db.conn).unwrap(), vec![a]);
    let raw: String = db
        .conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'ignored_volumes'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        v,
        json!([{"volumeKey": "vol-a", "label": "USB", "lastMountPoint": "E:\\", "ignoredAt": 10}])
    );
}

#[test]
fn t_ignvol_db_02_missing_key_means_empty_list() {
    let db = migrated_db();

    assert!(load_ignored_volumes(&db.conn).unwrap().is_empty());
}

#[test]
fn t_ignvol_db_03_corrupt_json_falls_back_to_empty_and_add_recovers() {
    let db = migrated_db();
    for raw in ["{not json", "{\"a\":1}", "\"x\"", "[{\"volumeKey\":1}]"] {
        db.conn.execute("DELETE FROM settings", []).unwrap();
        put_raw(&db.conn, raw);

        assert!(load_ignored_volumes(&db.conn).unwrap().is_empty(), "raw = {raw}");
    }
    // 깨진 값 위에 추가하면 정상 목록으로 복구된다
    add_ignored_volume(&db.conn, &vol("k", None, None, 1)).unwrap();
    assert_eq!(load_ignored_volumes(&db.conn).unwrap().len(), 1);
}

#[test]
fn t_ignvol_db_04_duplicate_add_dedupes_by_key_and_updates_label_and_mount() {
    // Arrange
    let db = migrated_db();
    add_ignored_volume(&db.conn, &vol("k1", Some("Old"), Some(r"E:\"), 10)).unwrap();
    add_ignored_volume(&db.conn, &vol("k2", Some("Other"), None, 11)).unwrap();

    // Act
    add_ignored_volume(&db.conn, &vol("k1", Some("New"), Some(r"F:\"), 99)).unwrap();

    // Assert: 항목 수·순서 유지, 라벨·마운트만 갱신, 처음 기억한 시각 유지
    let list = load_ignored_volumes(&db.conn).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], vol("k1", Some("New"), Some(r"F:\"), 10));
    assert_eq!(list[1].volume_key, "k2");
}

#[test]
fn t_ignvol_db_05_update_changes_only_label_and_mount_of_the_key() {
    // Arrange
    let db = migrated_db();
    add_ignored_volume(&db.conn, &vol("k1", Some("A"), Some(r"E:\"), 10)).unwrap();
    add_ignored_volume(&db.conn, &vol("k2", Some("B"), Some(r"G:\"), 11)).unwrap();

    // Act
    let changed = update_ignored_volume(&db.conn, "k1", Some("A2"), Some(r"H:\")).unwrap();

    // Assert
    assert!(changed);
    let list = load_ignored_volumes(&db.conn).unwrap();
    assert_eq!(list[0], vol("k1", Some("A2"), Some(r"H:\"), 10));
    assert_eq!(list[1], vol("k2", Some("B"), Some(r"G:\"), 11));
}

#[test]
fn t_ignvol_db_06_remove_and_update_of_unknown_key_are_no_ops() {
    // Arrange
    let db = migrated_db();
    add_ignored_volume(&db.conn, &vol("k1", None, None, 10)).unwrap();

    // Act / Assert
    assert!(!remove_ignored_volume(&db.conn, "nope").unwrap());
    assert!(!update_ignored_volume(&db.conn, "nope", Some("x"), None).unwrap());
    assert_eq!(load_ignored_volumes(&db.conn).unwrap().len(), 1);
    assert!(remove_ignored_volume(&db.conn, "k1").unwrap());
    assert!(load_ignored_volumes(&db.conn).unwrap().is_empty());
    assert!(!remove_ignored_volume(&db.conn, "k1").unwrap(), "second remove");
}

#[test]
fn t_ignvol_db_07_key_is_invisible_to_settings_load_and_patch() {
    // Arrange
    let mut db = migrated_db();
    add_ignored_volume(&db.conn, &vol("k1", None, None, 10)).unwrap();

    // Act / Assert: 설정 로드에 경고·영향이 없고, 패치로는 쓸 수 없다
    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(loaded, settings::Settings::default());
    assert!(warnings.is_empty());
    for field in ["ignored_volumes", "ignoredVolumes"] {
        let mut patch = Map::new();
        patch.insert(field.into(), json!([]));
        assert!(matches!(
            settings::update(&mut db.conn, &patch),
            Err(settings::SettingsError::Unknown(_))
        ));
    }
    assert_eq!(load_ignored_volumes(&db.conn).unwrap().len(), 1, "state untouched");
}

#[test]
fn t_ignvol_db_08_unchanged_update_does_not_rewrite() {
    // Arrange
    let db = migrated_db();
    add_ignored_volume(&db.conn, &vol("k1", Some("A"), Some(r"E:\"), 10)).unwrap();

    // Act / Assert: 같은 값 반복 -> false (30초 점검마다 쓰지 않는다)
    for _ in 0..3 {
        assert!(!update_ignored_volume(&db.conn, "k1", Some("A"), Some(r"E:\")).unwrap());
    }
}
