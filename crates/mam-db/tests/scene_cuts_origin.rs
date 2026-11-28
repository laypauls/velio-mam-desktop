//! V30 `asset_scene_cuts.origin` · 자동 추출 옵션 OFF 정리 (T-scoff-01 ~ 06).
//! 분류: Business rule (출처 기록·수동 보존·승격) / Validation (CHECK) / Boundary (구 행·편집이 있는 행·실행 중 작업) /
//! Concurrency (정리 멱등) / 게이트 ON (true 유지) / 성능 (EXPLAIN).

mod common;
mod scene_support;

use common::{FixedClock, T0, TestDb, shipped_migrations};
use mam_db::scene_cuts::{
    self, RequestOptions, SCENE_PRIORITY_USER, SceneState, add_user_tx, backfill_batch_tx,
    drop_auto_pending_tx, enqueue_auto_tx, request_tx,
};
use mam_db::{migrate, migrate_with, open, settings};
use scene_support::{NOW, add_video, count, scene_jobs};
use serde_json::{Map, Value, json};
use tempfile::TempDir;

fn db_upto(version: i64) -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let list: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| i64::from(m.version()) <= version)
        .collect();
    migrate_with(&mut conn, &list, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn origin(db: &TestDb, id: i64) -> Option<String> {
    db.conn
        .query_row(
            "SELECT origin FROM asset_scene_cuts WHERE asset_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn rows(db: &TestDb) -> i64 {
    count(&db.conn, "SELECT COUNT(*) FROM asset_scene_cuts")
}

fn user_request(db: &TestDb, ids: &[i64]) {
    request_tx(
        &db.conn,
        ids,
        &RequestOptions::default(),
        SCENE_PRIORITY_USER,
        NOW,
    )
    .unwrap();
}

#[test]
fn t_scoff_01_v30_backfills_only_pending_rows_as_auto_and_rejects_unknown_origin() {
    // Arrange: V29 시점 DB 에 상태별 구 행
    let mut db = db_upto(29);
    for id in 1..=4 {
        add_video(&db.conn, id, None, None);
    }
    for (id, state) in [
        (1, "pending"),
        (2, "done"),
        (3, "failed"),
        (4, "unsupported"),
    ] {
        db.conn
            .execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, ?2, ?3)",
                (id, state, T0),
            )
            .unwrap();
    }

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![30, 31]);
    assert_eq!(origin(&db, 1).as_deref(), Some("auto"));
    for id in 2..=4 {
        assert_eq!(origin(&db, id), None, "{id}");
    }
    add_video(&db.conn, 5, None, None);
    let bad = db.conn.execute(
        "INSERT INTO asset_scene_cuts (asset_id, state, updated_at, origin) VALUES (5, 'pending', 1, 'bogus')",
        [],
    );
    assert!(bad.is_err());
}

#[test]
fn t_scoff_02_registration_paths_record_their_origin() {
    // Arrange
    let db = common::migrated_db();
    for id in 1..=3 {
        add_video(&db.conn, id, None, None);
    }

    // Act: 1 = 자동, 2 = 사용자 요청, 3 = 백필 (1·2 는 행이 있어 백필 대상이 아니다)
    enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    user_request(&db, &[2]);
    let batch = backfill_batch_tx(&db.conn, 0, 10, NOW).unwrap();

    // Assert
    assert_eq!(batch.queued, 1);
    assert_eq!(origin(&db, 1).as_deref(), Some("auto"));
    assert_eq!(origin(&db, 2).as_deref(), Some("manual"));
    assert_eq!(origin(&db, 3).as_deref(), Some("manual"));
}

#[test]
fn t_scoff_03_manual_request_upgrades_a_pending_auto_row_so_cleanup_keeps_it() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    enqueue_auto_tx(&db.conn, 1, NOW).unwrap();

    // Act
    user_request(&db, &[1]);
    let dropped = drop_auto_pending_tx(&db.conn, None, NOW).unwrap();

    // Assert: 사용자 요청이 우선 - 행·작업 보존, 우선순위 승격
    assert_eq!(dropped, 0);
    assert_eq!(origin(&db, 1).as_deref(), Some("manual"));
    assert_eq!(
        scene_jobs(&db.conn, 1),
        vec![(SCENE_PRIORITY_USER, "pending".to_owned())]
    );
}

#[test]
fn t_scoff_04_drop_auto_pending_removes_auto_rows_and_queued_jobs_but_keeps_manual_running_and_edited()
 {
    // Arrange: 1 자동 대기, 2 수동 대기, 3 자동 + 실행 중 작업, 4 자동 대기 + 사용자 컷, 5 구 완료 행
    let db = common::migrated_db();
    for id in 1..=5 {
        add_video(&db.conn, id, None, None);
    }
    enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    user_request(&db, &[2]);
    enqueue_auto_tx(&db.conn, 3, NOW).unwrap();
    db.conn
        .execute("UPDATE jobs SET status = 'running' WHERE asset_id = 3", [])
        .unwrap();
    enqueue_auto_tx(&db.conn, 4, NOW).unwrap();
    add_user_tx(&db.conn, 4, 5_000, None, None, NOW).unwrap();
    db.conn
        .execute(
            "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (5, 'done', 1)",
            [],
        )
        .unwrap();

    // Act
    let n = drop_auto_pending_tx(&db.conn, None, NOW + 1).unwrap();
    let again = drop_auto_pending_tx(&db.conn, None, NOW + 2).unwrap();

    // Assert
    assert_eq!(n, 3, "1 삭제 + 3 삭제 + 4 done 전환");
    assert_eq!(again, 0, "멱등");
    assert!(scene_cuts::get(&db.conn, 1).unwrap().is_none());
    assert!(scene_jobs(&db.conn, 1).is_empty());
    let manual = scene_cuts::get(&db.conn, 2).unwrap().unwrap();
    assert_eq!(manual.state, SceneState::Pending);
    assert_eq!(scene_jobs(&db.conn, 2).len(), 1);
    // 실행 중 작업은 건드리지 않는다 (실행기가 시작 시 스스로 끝낸다)
    assert_eq!(scene_jobs(&db.conn, 3), vec![(60, "running".to_owned())]);
    let edited = scene_cuts::get(&db.conn, 4).unwrap().unwrap();
    assert_eq!((edited.state, edited.user), (SceneState::Done, vec![5_000]));
    assert!(scene_jobs(&db.conn, 4).is_empty());
    let old = scene_cuts::get(&db.conn, 5).unwrap().unwrap();
    assert_eq!(old.state, SceneState::Done);
}

#[test]
fn t_scoff_04b_scoped_drop_touches_only_that_asset() {
    // Arrange
    let db = common::migrated_db();
    for id in 1..=2 {
        add_video(&db.conn, id, None, None);
        enqueue_auto_tx(&db.conn, id, NOW).unwrap();
    }

    // Act
    let n = drop_auto_pending_tx(&db.conn, Some(1), NOW).unwrap();

    // Assert
    assert_eq!(n, 1);
    assert_eq!(rows(&db), 1);
    assert!(scene_cuts::get(&db.conn, 2).unwrap().is_some());
    assert_eq!(scene_jobs(&db.conn, 2).len(), 1);
}

#[test]
fn t_scoff_05_settings_save_true_to_false_clears_auto_pending_in_the_same_transaction() {
    // Arrange: 옵션 ON + 자동 대기 1 + 수동 대기 1
    let mut db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    add_video(&db.conn, 2, None, None);
    settings::update(&mut db.conn, &patch(json!({ "sceneDetectAuto": true }))).unwrap();
    enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    user_request(&db, &[2]);

    // Act / Assert: ON 유지(게이트 ON)에서는 정리하지 않는다
    settings::update(&mut db.conn, &patch(json!({ "sceneDetectAuto": true }))).unwrap();
    assert_eq!(rows(&db), 2);

    // Act: true -> false
    let s = settings::update(&mut db.conn, &patch(json!({ "sceneDetectAuto": false }))).unwrap();

    // Assert
    assert!(!s.scene_detect_auto);
    assert!(scene_cuts::get(&db.conn, 1).unwrap().is_none());
    assert!(scene_jobs(&db.conn, 1).is_empty());
    assert!(scene_cuts::get(&db.conn, 2).unwrap().is_some());
    assert_eq!(scene_jobs(&db.conn, 2).len(), 1);
}

#[test]
fn t_scoff_05b_invalid_patch_or_false_to_false_does_not_clear_anything() {
    // Arrange: 옵션 OFF 인 채 남은 자동 대기 행 (구 데이터)
    let mut db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    enqueue_auto_tx(&db.conn, 1, NOW).unwrap();

    // Act: false -> false (전이 아님) / 잘못된 값 (거부)
    settings::update(&mut db.conn, &patch(json!({ "sceneDetectAuto": false }))).unwrap();
    let bad = settings::update(
        &mut db.conn,
        &patch(json!({ "sceneDetectAuto": false, "sceneDetectSensitivity": "bogus" })),
    );

    // Assert
    assert!(bad.is_err());
    assert_eq!(rows(&db), 1);
    assert_eq!(scene_jobs(&db.conn, 1).len(), 1);
}

#[test]
fn t_scoff_06_cleanup_selects_pending_auto_rows_through_the_partial_index() {
    // Arrange
    let db = common::migrated_db();
    let sql = "EXPLAIN QUERY PLAN SELECT asset_id FROM asset_scene_cuts
               WHERE state = 'pending' AND origin = 'auto'";

    // Act
    let plan: Vec<String> = db
        .conn
        .prepare(sql)
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // Assert
    assert!(
        plan.iter()
            .any(|d| d.contains("idx_asset_scene_cuts_pending")),
        "{plan:?}"
    );
}
