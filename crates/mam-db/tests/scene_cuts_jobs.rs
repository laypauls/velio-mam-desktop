//! 장면 검출 작업 등록 (T-scene-11, 12).
//!
//! 분류: Business rule (요청 queued/skipped, 승격, 실행 중 작업) / Boundary (백필 1000 경계·0개) /
//! Concurrency (멱등 — 같은 요청·배치 반복)

mod common;
mod scene_support;

use mam_db::SceneSensitivity;
use mam_db::scene_cuts::{
    self, BACKFILL_BATCH, RequestOptions, SCENE_PRIORITY_BACKGROUND, SCENE_PRIORITY_USER,
    SceneState, backfill_batch_tx, backfill_count, enqueue_auto_tx, replace_auto_tx, request_tx,
    requeue_pending_tx, set_outcome_tx,
};
use scene_support::{NOW, add_asset, add_video, count, scene_jobs};

const USER: i64 = SCENE_PRIORITY_USER;
const BG: i64 = SCENE_PRIORITY_BACKGROUND;

fn opts() -> RequestOptions {
    RequestOptions::default()
}

#[test]
fn t_scene_11_request_counts_queued_and_skipped_and_registers_user_priority_jobs() {
    // Arrange: 1 영상, 2 이미지, 3 비활성 영상, 4 자동 추출 완료 영상, 5 미지원 영상, 99 없는 자산
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    add_asset(&db.conn, 2, "image", "active");
    add_asset(&db.conn, 3, "video", "superseded");
    add_video(&db.conn, 4, None, None);
    replace_auto_tx(&db.conn, 4, &[3000], "medium", None, NOW).unwrap();
    add_video(&db.conn, 5, None, None);
    set_outcome_tx(&db.conn, 5, SceneState::Unsupported, None, NOW).unwrap();

    // Act
    let out = request_tx(&db.conn, &[1, 2, 3, 4, 5, 99], &opts(), USER, NOW).unwrap();

    // Assert
    assert_eq!((out.queued, out.skipped), (1, 5));
    assert_eq!(scene_jobs(&db.conn, 1), vec![(USER, "pending".to_owned())]);
    for skipped in [2, 3, 4, 5, 99] {
        assert!(scene_jobs(&db.conn, skipped).is_empty(), "{skipped}");
    }
    let r = scene_cuts::get(&db.conn, 1).unwrap().unwrap();
    assert_eq!(r.state, SceneState::Pending);
    assert!(scene_cuts::get(&db.conn, 2).unwrap().is_none(), "건너뛴 자산은 행도 만들지 않는다");
    // 빈 목록
    let none = request_tx(&db.conn, &[], &opts(), USER, NOW).unwrap();
    assert_eq!((none.queued, none.skipped), (0, 0));
}

#[test]
fn t_scene_11_redetect_requeues_done_and_unsupported_and_records_the_requested_sensitivity() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    replace_auto_tx(&db.conn, 1, &[3000], "medium", None, NOW).unwrap();
    add_video(&db.conn, 2, None, None);
    set_outcome_tx(&db.conn, 2, SceneState::Unsupported, None, NOW).unwrap();
    let redo = RequestOptions { redetect: true, restore_suppressed: false, sensitivity: Some(SceneSensitivity::High) };

    // Act
    let out = request_tx(&db.conn, &[1, 2], &redo, USER, NOW + 1).unwrap();

    // Assert: 기존 자동 컷은 새 결과가 올 때까지 유지, 요청 민감도·상태만 바뀐다
    assert_eq!((out.queued, out.skipped), (2, 0));
    let r = scene_cuts::get(&db.conn, 1).unwrap().unwrap();
    assert_eq!(r.state, SceneState::Pending);
    assert_eq!(r.requested_sensitivity.as_deref(), Some("high"));
    assert_eq!(r.sensitivity.as_deref(), Some("medium"));
    assert_eq!(r.auto, vec![3000]);
    assert_eq!(scene_jobs(&db.conn, 2).len(), 1);
}

#[test]
fn t_scene_11_request_on_a_user_only_row_still_extracts() {
    // Arrange: 사용자 컷만 있는 행(state=done, sensitivity=NULL)은 "추출 전" 이다
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    scene_cuts::add_user_tx(&db.conn, 1, 5000, None, None, NOW).unwrap();

    // Act
    let out = request_tx(&db.conn, &[1], &opts(), USER, NOW).unwrap();

    // Assert
    assert_eq!((out.queued, out.skipped), (1, 0));
    assert_eq!(scene_cuts::get(&db.conn, 1).unwrap().unwrap().user, vec![5000]);
}

#[test]
fn t_scene_11_request_promotes_a_waiting_background_job_to_45_and_is_idempotent() {
    // Arrange: 백필이 60 으로 등록해 둔 작업
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    backfill_batch_tx(&db.conn, 0, BACKFILL_BATCH, NOW).unwrap();
    assert_eq!(scene_jobs(&db.conn, 1), vec![(BG, "pending".to_owned())]);

    // Act
    let first = request_tx(&db.conn, &[1], &opts(), USER, NOW + 1).unwrap();
    let second = request_tx(&db.conn, &[1], &opts(), USER, NOW + 2).unwrap();
    let lower = request_tx(&db.conn, &[1], &opts(), BG, NOW + 3).unwrap();

    // Assert: 하나의 작업만, 45 로 승격, 낮은 우선순위 요청이 되돌리지 않는다
    assert_eq!((first.queued, second.queued, lower.queued), (1, 1, 1));
    assert_eq!(scene_jobs(&db.conn, 1), vec![(USER, "pending".to_owned())]);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_scene_cuts"), 1);
}

#[test]
fn t_scene_11_running_and_failed_jobs_are_handled_without_duplicates_or_unique_violations() {
    // Arrange: 1 은 실행 중, 2 는 실패 작업이 남아 있음
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    add_video(&db.conn, 2, None, None);
    backfill_batch_tx(&db.conn, 0, BACKFILL_BATCH, NOW).unwrap();
    db.conn
        .execute("UPDATE jobs SET status = 'running' WHERE asset_id = 1", [])
        .unwrap();
    db.conn
        .execute("UPDATE jobs SET status = 'failed', attempts = 5, last_error = 'x' WHERE asset_id = 2", [])
        .unwrap();

    // Act
    request_tx(&db.conn, &[1, 2], &opts(), USER, NOW + 1).unwrap();

    // Assert: 실행 중인 작업이 있으면 새로 올리지 않고, 실패 작업은 새 대기 작업으로 대체된다
    assert_eq!(scene_jobs(&db.conn, 1), vec![(BG, "running".to_owned())]);
    assert_eq!(scene_jobs(&db.conn, 2), vec![(USER, "pending".to_owned())]);

    // Act: 재검출 요청은 실행 중이어도 다음 작업을 올린다
    let redo = RequestOptions { redetect: true, ..opts() };
    request_tx(&db.conn, &[1], &redo, USER, NOW + 2).unwrap();
    assert_eq!(
        scene_jobs(&db.conn, 1),
        vec![(BG, "running".to_owned()), (USER, "pending".to_owned())]
    );
}

#[test]
fn t_scene_12_backfill_registers_only_active_videos_without_a_row_in_batches() {
    // Arrange: 영상 2,500 + 이미지·비활성 영상·실패 행·사용자 컷 행(둘 다 "행 있음")
    let db = common::migrated_db();
    db.conn
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2500)
             INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             SELECT i, 'video', 1, 0, 0, 0 FROM n;",
        )
        .unwrap();
    add_asset(&db.conn, 3000, "image", "active");
    add_asset(&db.conn, 3001, "video", "deleted");
    add_asset(&db.conn, 3002, "video", "superseded");
    set_outcome_tx(&db.conn, 1, SceneState::Failed, Some("io"), NOW).unwrap();
    scene_cuts::add_user_tx(&db.conn, 2, 5000, None, None, NOW).unwrap();
    let expected = 2500 - 2;
    assert_eq!(backfill_count(&db.conn).unwrap(), expected);

    // Act: 키셋 배치 반복
    let mut after = 0;
    let mut batches = Vec::new();
    loop {
        let tx = db.conn.unchecked_transaction().unwrap();
        let b = backfill_batch_tx(&tx, after, BACKFILL_BATCH, NOW).unwrap();
        tx.commit().unwrap();
        batches.push(b.queued);
        match b.last_id {
            Some(id) => after = id,
            None => break,
        }
    }

    // Assert: 1000 + 1000 + 498 + 마지막 빈 배치, 합계 = count, 우선순위 60·pending, 이후 count 0
    assert_eq!(batches, vec![1000, 1000, 498, 0]);
    assert_eq!(batches.iter().map(|b| i64::from(*b)).sum::<i64>(), expected);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM jobs WHERE kind = 'scene'"), expected);
    assert_eq!(
        count(&db.conn, &format!("SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND priority = {BG} AND status = 'pending'")),
        expected
    );
    assert_eq!(
        count(&db.conn, "SELECT COUNT(*) FROM asset_scene_cuts WHERE state = 'pending'"),
        expected
    );
    assert_eq!(backfill_count(&db.conn).unwrap(), 0);
    // 멱등: 다시 돌려도 아무것도 늘지 않고, 실패 행(1)은 건드리지 않는다
    let again = backfill_batch_tx(&db.conn, 0, BACKFILL_BATCH, NOW + 1).unwrap();
    assert_eq!((again.queued, again.last_id), (0, None));
    let failed = scene_cuts::get(&db.conn, 1).unwrap().unwrap();
    assert_eq!(failed.state, SceneState::Failed);
    assert!(scene_jobs(&db.conn, 1).is_empty());
    assert!(scene_jobs(&db.conn, 2).is_empty());
}

#[test]
fn t_scene_12_backfill_boundaries_exactly_one_batch_and_zero_targets() {
    // Arrange: 0 개
    let db = common::migrated_db();
    assert_eq!(backfill_count(&db.conn).unwrap(), 0);
    let empty = backfill_batch_tx(&db.conn, 0, BACKFILL_BATCH, NOW).unwrap();
    assert_eq!((empty.queued, empty.last_id), (0, None));
    db.conn
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000)
             INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             SELECT i, 'video', 1, 0, 0, 0 FROM n;",
        )
        .unwrap();

    // Act: 정확히 1000 개
    let first = backfill_batch_tx(&db.conn, 0, BACKFILL_BATCH, NOW).unwrap();
    let second = backfill_batch_tx(&db.conn, first.last_id.unwrap(), BACKFILL_BATCH, NOW).unwrap();

    // Assert
    assert_eq!((first.queued, first.last_id), (1000, Some(1000)));
    assert_eq!((second.queued, second.last_id), (0, None));
    assert_eq!(backfill_count(&db.conn).unwrap(), 0);
}

#[test]
fn t_scene_12_requeue_pending_only_registers_active_assets_without_a_live_job() {
    // Arrange: 1 작업 없음, 2 대기 작업 있음, 3 실행 중, 4 비활성(작업 없음), 5 failed 상태 행
    let db = common::migrated_db();
    for id in 1..=3 {
        add_video(&db.conn, id, None, None);
    }
    add_asset(&db.conn, 4, "video", "superseded");
    add_video(&db.conn, 5, None, None);
    for id in 1..=4 {
        db.conn
            .execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, 'pending', 0)",
                [id],
            )
            .unwrap();
    }
    set_outcome_tx(&db.conn, 5, SceneState::Failed, None, NOW).unwrap();
    db.conn
        .execute_batch(
            "INSERT INTO jobs (kind, asset_id, priority, created_at, updated_at) VALUES ('scene', 2, 45, 0, 0);
             INSERT INTO jobs (kind, asset_id, priority, status, created_at, updated_at) VALUES ('scene', 3, 60, 'running', 0, 0);",
        )
        .unwrap();

    // Act
    let n = requeue_pending_tx(&db.conn, NOW).unwrap();
    let again = requeue_pending_tx(&db.conn, NOW).unwrap();

    // Assert
    assert_eq!((n, again), (1, 0));
    assert_eq!(scene_jobs(&db.conn, 1), vec![(BG, "pending".to_owned())]);
    assert_eq!(scene_jobs(&db.conn, 2), vec![(USER, "pending".to_owned())], "기존 작업 우선순위 유지");
    assert_eq!(scene_jobs(&db.conn, 3).len(), 1);
    assert!(scene_jobs(&db.conn, 4).is_empty());
    assert!(scene_jobs(&db.conn, 5).is_empty());
}

#[test]
fn t_scene_12_enqueue_auto_creates_a_pending_row_and_job_only_once() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    add_video(&db.conn, 2, None, None);
    replace_auto_tx(&db.conn, 2, &[3000], "medium", None, NOW).unwrap();

    // Act
    let first = enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    let second = enqueue_auto_tx(&db.conn, 1, NOW + 1).unwrap();
    let done_already = enqueue_auto_tx(&db.conn, 2, NOW).unwrap();

    // Assert: 메타 재실행 등으로 다시 불려도 중복 없음, 이미 결과가 있는 자산은 건드리지 않음
    assert_eq!((first, second, done_already), (true, false, false));
    assert_eq!(scene_jobs(&db.conn, 1), vec![(BG, "pending".to_owned())]);
    assert!(scene_jobs(&db.conn, 2).is_empty());
    assert_eq!(scene_cuts::get(&db.conn, 2).unwrap().unwrap().auto, vec![3000]);
}
