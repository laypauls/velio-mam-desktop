//! 작업 일시정지 영속 상태 `job_pause` 통합 테스트 (T-jobpause-db-01 ~ 04, T-jobpause-07).
//!
//! 분류: Happy (01 왕복) / Boundary (02 키 없음) / Validation (03 깨진 JSON -> 해제, 07 설정 계층에 비노출).

mod common;

use common::migrated_db;
use mam_db::job_pause::{JobPauseState, load_job_pause, save_job_pause};
use mam_db::settings;
use serde_json::{Map, json};

fn put_raw(conn: &rusqlite::Connection, raw: &str) {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('job_pause', ?1)",
        [raw],
    )
    .unwrap();
}

#[test]
fn t_jobpause_db_01_save_then_load_round_trips_and_overwrites() {
    // Arrange
    let db = migrated_db();
    let first = JobPauseState {
        all: false,
        kinds: vec!["hash".into(), "ai".into()],
    };

    // Act / Assert
    save_job_pause(&db.conn, &first).unwrap();
    assert_eq!(load_job_pause(&db.conn).unwrap(), first);
    let second = JobPauseState {
        all: true,
        kinds: vec![],
    };
    save_job_pause(&db.conn, &second).unwrap();
    assert_eq!(load_job_pause(&db.conn).unwrap(), second);
}

#[test]
fn t_jobpause_db_02_missing_key_means_not_paused() {
    let db = migrated_db();

    assert_eq!(load_job_pause(&db.conn).unwrap(), JobPauseState::default());
}

#[test]
fn t_jobpause_db_03_corrupt_json_falls_back_to_not_paused() {
    let db = migrated_db();
    for raw in ["{not json", "[1,2]", "\"x\"", "{\"all\":\"yes\"}"] {
        db.conn.execute("DELETE FROM settings", []).unwrap();
        put_raw(&db.conn, raw);

        assert_eq!(
            load_job_pause(&db.conn).unwrap(),
            JobPauseState::default(),
            "raw = {raw}"
        );
    }
}

#[test]
fn t_jobpause_07_key_is_invisible_to_settings_load_and_patch() {
    // Arrange: 저장된 상태가 있어도
    let mut db = migrated_db();
    save_job_pause(
        &db.conn,
        &JobPauseState {
            all: true,
            kinds: vec![],
        },
    )
    .unwrap();

    // Act / Assert: 설정 로드에 경고·영향이 없고, 패치로는 쓸 수 없다
    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(loaded, settings::Settings::default());
    assert!(warnings.is_empty());
    for field in ["job_pause", "jobPause"] {
        let mut patch = Map::new();
        patch.insert(field.into(), json!({"all": false, "kinds": []}));
        assert!(matches!(
            settings::update(&mut db.conn, &patch),
            Err(settings::SettingsError::Unknown(_))
        ));
    }
    assert!(load_job_pause(&db.conn).unwrap().all, "state untouched");
}
