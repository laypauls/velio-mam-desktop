//! 장면 컷 편집·자동 결과 교체 (T-scene-07 ~ 10, 14).
//!
//! 분류: Business rule (숨김·사용자 컷 허용 오차, 제거·숨김 기억, 멱등) / Validation (add 범위·상한) /
//! Concurrency (검출 결과 교체와 동시 사용자 편집) / 불변 (revision·search_outbox)

mod common;
mod scene_support;

use mam_db::open;
use mam_db::scene_cuts::{
    self, CutSource, MAX_USER_CUTS, RequestOptions, SceneError, SceneState, add_user_tx,
    clear_auto_tx, remove_tx, replace_auto_tx, request_tx, set_outcome_tx,
};
use scene_support::{NOW, add_video, count};

fn row(conn: &rusqlite::Connection, id: i64) -> scene_cuts::SceneRow {
    scene_cuts::get(conn, id).unwrap().unwrap()
}

#[test]
fn t_scene_07_replace_auto_excludes_suppressed_and_user_cuts_within_the_frame_tolerance() {
    // Arrange: fps 30 → 허용 오차 33ms. 숨김 10,000 / 사용자 20,000
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(30.0));
    db.conn
        .execute(
            "INSERT INTO asset_scene_cuts (asset_id, state, user_cuts, suppressed, updated_at)
             VALUES (1, 'pending', ?1, ?2, 0)",
            rusqlite::params![
                scene_cuts::encode(&[20_000]).unwrap(),
                scene_cuts::encode(&[10_000]).unwrap()
            ],
        )
        .unwrap();

    // Act
    let n = replace_auto_tx(
        &db.conn,
        1,
        &[5000, 9966, 9967, 10_033, 10_034, 19_967, 20_033, 20_034],
        "medium",
        Some(30.0),
        NOW,
    )
    .unwrap();

    // Assert: ±33ms 안(9967..=10033, 19967..=20033)만 빠진다 — 경계 34ms 는 남는다
    let r = row(&db.conn, 1);
    assert_eq!(r.auto, vec![5000, 9966, 10_034, 20_034]);
    assert_eq!(n, 4);
    assert_eq!(r.user, vec![20_000], "사용자 컷 보존");
    assert_eq!(r.suppressed, vec![10_000], "숨김 보존");
    assert_eq!(r.state, SceneState::Done);
    assert_eq!(r.sensitivity.as_deref(), Some("medium"));
    assert_eq!(r.detector_version, Some(scene_cuts::SCENE_DETECTOR_VERSION));
    assert!(r.requested_sensitivity.is_none() && r.error.is_none());
    assert_eq!(r.updated_at, NOW);
}

#[test]
fn t_scene_07_unknown_fps_uses_the_100ms_tolerance() {
    // Arrange: fps 0·없음·NaN → 100ms
    for fps in [None, Some(0.0), Some(f64::NAN)] {
        let db = common::migrated_db();
        add_video(&db.conn, 1, None, None);
        add_user_tx(&db.conn, 1, 10_000, fps, None, NOW).unwrap();

        // Act
        replace_auto_tx(&db.conn, 1, &[9900, 9899, 10_100, 10_101], "low", fps, NOW).unwrap();

        // Assert
        assert_eq!(row(&db.conn, 1).auto, vec![9899, 10_101], "{fps:?}");
    }
}

#[test]
fn t_scene_07_restore_suppressed_brings_hidden_cuts_back_on_the_next_detection() {
    // Arrange: 자동 컷 2개 중 하나를 사용자가 지움 → 재추출해도 미복원
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    replace_auto_tx(&db.conn, 1, &[3000, 8000], "medium", Some(25.0), NOW).unwrap();
    assert!(remove_tx(&db.conn, 1, 3000, CutSource::Auto, Some(25.0), NOW).unwrap());
    replace_auto_tx(&db.conn, 1, &[3000, 8000], "high", Some(25.0), NOW).unwrap();
    assert_eq!(row(&db.conn, 1).auto, vec![8000]);

    // Act: 되살리기 체크 + 재검출 요청 후 결과 교체
    let opts = RequestOptions {
        redetect: true,
        restore_suppressed: true,
        sensitivity: None,
    };
    request_tx(&db.conn, &[1], &opts, 45, NOW).unwrap();
    assert!(row(&db.conn, 1).suppressed.is_empty());
    replace_auto_tx(&db.conn, 1, &[3000, 8000], "high", Some(25.0), NOW).unwrap();

    // Assert
    assert_eq!(row(&db.conn, 1).auto, vec![3000, 8000]);
}

#[test]
fn t_scene_07_replace_rejects_unknown_sensitivity_and_writes_nothing() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);

    // Act
    let err = replace_auto_tx(&db.conn, 1, &[1000], "extreme", None, NOW).unwrap_err();

    // Assert
    assert!(matches!(err, SceneError::InvalidInput("sensitivity")));
    assert!(scene_cuts::get(&db.conn, 1).unwrap().is_none());
}

#[test]
fn t_scene_08_add_user_validates_range_and_creates_a_done_row_without_fingerprint() {
    // Arrange: 길이 10,000ms
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(10_000), Some(25.0));

    // Act / Assert: 범위 검증
    for bad in [-1, 10_001, i64::from(u32::MAX) + 1] {
        let err = add_user_tx(&db.conn, 1, bad, Some(25.0), Some(10_000), NOW).unwrap_err();
        assert!(matches!(err, SceneError::InvalidInput("ms")), "{bad}");
    }
    assert!(
        scene_cuts::get(&db.conn, 1).unwrap().is_none(),
        "거부된 입력은 행을 만들지 않는다"
    );
    // 길이를 모르면 u32 범위까지, 경계값 0·길이 그대로는 허용
    assert!(add_user_tx(&db.conn, 1, 0, Some(25.0), Some(10_000), NOW).unwrap());
    assert!(add_user_tx(&db.conn, 1, 10_000, Some(25.0), Some(10_000), NOW).unwrap());
    assert!(add_user_tx(&db.conn, 1, 4_000_000_000, None, None, NOW).unwrap());
    let r = row(&db.conn, 1);
    assert_eq!(r.user, vec![0, 10_000, 4_000_000_000]);
    assert_eq!(r.state, SceneState::Done);
    assert!(
        r.sensitivity.is_none() && r.detector_version.is_none(),
        "추출 전 표시 유지용"
    );
    assert!(r.auto.is_empty() && r.suppressed.is_empty());
}

#[test]
fn t_scene_08_add_user_is_a_noop_within_tolerance_of_any_existing_cut() {
    // Arrange: fps 25 → 40ms. 자동 5000, 사용자 8000
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    replace_auto_tx(&db.conn, 1, &[5000], "medium", Some(25.0), NOW).unwrap();
    assert!(add_user_tx(&db.conn, 1, 8000, Some(25.0), None, NOW).unwrap());

    // Act / Assert: 사용자·자동 컷 ±40ms 안은 무시 (멱등), 41ms 는 추가
    assert!(!add_user_tx(&db.conn, 1, 8000, Some(25.0), None, NOW + 1).unwrap());
    assert!(!add_user_tx(&db.conn, 1, 8040, Some(25.0), None, NOW + 1).unwrap());
    assert!(!add_user_tx(&db.conn, 1, 4960, Some(25.0), None, NOW + 1).unwrap());
    assert!(add_user_tx(&db.conn, 1, 5041, Some(25.0), None, NOW + 1).unwrap());
    let r = row(&db.conn, 1);
    assert_eq!(r.user, vec![5041, 8000], "정렬 유지");
    assert_eq!(r.updated_at, NOW + 1);
}

#[test]
fn t_scene_08_the_5001st_user_cut_is_rejected_and_the_first_5000_survive() {
    // Arrange: fps 없음(100ms 허용 오차)이라 1000ms 간격으로 5,000개
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    let tx = db.conn.unchecked_transaction().unwrap();
    for i in 0..MAX_USER_CUTS as i64 {
        assert!(add_user_tx(&tx, 1, i * 1000, None, None, NOW).unwrap());
    }
    tx.commit().unwrap();

    // Act
    let err = add_user_tx(&db.conn, 1, 9_000_000, None, None, NOW).unwrap_err();
    let dup = add_user_tx(&db.conn, 1, 1000, None, None, NOW).unwrap();

    // Assert: 상한 초과는 거부, 이미 있는 위치는 상한과 무관하게 no-op
    assert!(matches!(err, SceneError::InvalidInput("cuts")));
    assert!(!dup);
    assert_eq!(row(&db.conn, 1).user.len(), MAX_USER_CUTS);
}

#[test]
fn t_scene_09_remove_auto_hides_it_user_deletes_it_and_both_are_idempotent() {
    // Arrange: fps 없음 → 허용 오차 100ms
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    replace_auto_tx(&db.conn, 1, &[3000, 6000, 6060], "medium", None, NOW).unwrap();
    // select_auto 를 거치지 않았으므로 6000·6060 둘 다 저장돼 "가장 가까운 1개" 선택을 볼 수 있다
    add_user_tx(&db.conn, 1, 9000, None, None, NOW).unwrap();

    // Act / Assert: 자동 — 가장 가까운 1개만 지우고 숨김에 기억
    assert!(remove_tx(&db.conn, 1, 6050, CutSource::Auto, None, NOW + 1).unwrap());
    let r = row(&db.conn, 1);
    assert_eq!(r.auto, vec![3000, 6000]);
    assert_eq!(r.suppressed, vec![6060]);
    // 이미 지운 위치 근처에 더 지울 컷이 없으면 no-op (6000 은 오차 100ms 밖)
    assert!(!remove_tx(&db.conn, 1, 6200, CutSource::Auto, None, NOW + 2).unwrap());
    assert_eq!(row(&db.conn, 1).suppressed, vec![6060]);
    // 사용자 — 삭제만, 숨김에 남지 않음
    assert!(remove_tx(&db.conn, 1, 9040, CutSource::User, None, NOW + 3).unwrap());
    let r = row(&db.conn, 1);
    assert!(r.user.is_empty());
    assert_eq!(r.suppressed, vec![6060]);
    assert!(!remove_tx(&db.conn, 1, 9000, CutSource::User, None, NOW + 4).unwrap());
    // 오차 밖·출처 불일치·행 없음은 no-op
    assert!(!remove_tx(&db.conn, 1, 3101, CutSource::Auto, None, NOW + 5).unwrap());
    assert!(!remove_tx(&db.conn, 1, 3000, CutSource::User, None, NOW + 5).unwrap());
    add_video(&db.conn, 2, None, None);
    assert!(!remove_tx(&db.conn, 2, 3000, CutSource::Auto, None, NOW + 5).unwrap());
    assert_eq!(row(&db.conn, 1).auto, vec![3000, 6000]);
}

#[test]
fn t_scene_09_clear_auto_hides_everything_and_a_redetect_does_not_revive_it() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    replace_auto_tx(&db.conn, 1, &[2000, 4000, 6000], "medium", Some(25.0), NOW).unwrap();
    add_user_tx(&db.conn, 1, 5000, Some(25.0), None, NOW).unwrap();

    // Act
    let cleared = clear_auto_tx(&db.conn, 1, NOW + 1).unwrap();
    let again = clear_auto_tx(&db.conn, 1, NOW + 2).unwrap();
    replace_auto_tx(
        &db.conn,
        1,
        &[2000, 4000, 6000, 7000],
        "high",
        Some(25.0),
        NOW + 3,
    )
    .unwrap();

    // Assert
    assert_eq!((cleared, again), (3, 0));
    let r = row(&db.conn, 1);
    assert_eq!(r.suppressed, vec![2000, 4000, 6000]);
    assert_eq!(r.auto, vec![7000], "새 점만 나온다");
    assert_eq!(r.user, vec![5000]);
    assert_eq!(clear_auto_tx(&db.conn, 99, NOW).unwrap(), 0, "행 없음");
}

#[test]
fn t_scene_09_failed_and_unsupported_outcomes_keep_cuts_and_store_only_fixed_error_codes() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    replace_auto_tx(&db.conn, 1, &[3000], "medium", None, NOW).unwrap();

    // Act
    set_outcome_tx(&db.conn, 1, SceneState::Failed, Some("timeout"), NOW + 1).unwrap();
    let failed = row(&db.conn, 1);
    set_outcome_tx(
        &db.conn,
        1,
        SceneState::Failed,
        Some("C:\\secret\\a.mp4: boom"),
        NOW + 2,
    )
    .unwrap();
    let sanitized = row(&db.conn, 1);
    set_outcome_tx(&db.conn, 1, SceneState::Unsupported, None, NOW + 3).unwrap();
    let unsupported = row(&db.conn, 1);
    let bad_state = set_outcome_tx(&db.conn, 1, SceneState::Done, None, NOW).unwrap_err();

    // Assert
    assert_eq!(
        (failed.state, failed.error.as_deref()),
        (SceneState::Failed, Some("timeout"))
    );
    assert_eq!(failed.auto, vec![3000], "실패해도 컷 유지");
    assert_eq!(
        sanitized.error.as_deref(),
        Some("other"),
        "경로·메시지는 저장하지 않는다"
    );
    assert_eq!(
        (unsupported.state, unsupported.error),
        (SceneState::Unsupported, None)
    );
    assert!(matches!(bad_state, SceneError::InvalidInput("state")));
}

#[test]
fn t_scene_10_replace_auto_keeps_a_user_edit_made_while_detection_was_running() {
    // Arrange: 두 연결 — A 가 "검출 중", B 는 사용자 편집
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    scene_cuts::enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    let user_conn = open(&db.db_path).unwrap();

    // Act: 검출이 끝나기 전에 사용자가 컷 추가·자동 컷 삭제 (먼저 한 번 등록돼 있던 자동 컷)
    add_user_tx(&user_conn, 1, 10_000, Some(25.0), Some(60_000), NOW + 1).unwrap();
    add_user_tx(&user_conn, 1, 30_000, Some(25.0), Some(60_000), NOW + 1).unwrap();
    // 이후 검출 결과가 도착: 10,000 근처 자동 컷은 사용자 컷과 겹쳐 빠진다
    let n = replace_auto_tx(
        &db.conn,
        1,
        &[5000, 10_020, 20_000],
        "medium",
        Some(25.0),
        NOW + 2,
    )
    .unwrap();

    // Assert
    let r = row(&db.conn, 1);
    assert_eq!(n, 2);
    assert_eq!(r.auto, vec![5000, 20_000]);
    assert_eq!(r.user, vec![10_000, 30_000]);
}

#[test]
fn t_scene_10_concurrent_user_adds_and_auto_replacements_never_lose_user_cuts() {
    // Arrange: 같은 파일 DB 에 연결을 따로 연 두 스레드가 경쟁 (BEGIN IMMEDIATE + busy timeout)
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    scene_cuts::enqueue_auto_tx(&db.conn, 1, NOW).unwrap();
    let path = db.db_path.clone();
    let adder = {
        let path = path.clone();
        std::thread::spawn(move || {
            let mut c = open(&path).unwrap();
            for i in 0..40i64 {
                let tx = c
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                assert!(scene_cuts::add_user_tx(&tx, 1, 1000 + i * 2000, None, None, NOW).unwrap());
                tx.commit().unwrap();
            }
        })
    };
    let replacer = std::thread::spawn(move || {
        let mut c = open(&path).unwrap();
        for round in 0..40i64 {
            let tx = c
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let auto: Vec<i64> = (0..30).map(|k| 500 + k * 2000 + round).collect();
            scene_cuts::replace_auto_tx(&tx, 1, &auto, "medium", None, NOW).unwrap();
            tx.commit().unwrap();
        }
    });

    // Act
    adder.join().unwrap();
    replacer.join().unwrap();

    // Assert: 모든 사용자 컷이 남고 자동 컷과 허용 오차 안에서 겹치지 않는다
    let r = row(&db.conn, 1);
    let expect: Vec<i64> = (0..40).map(|i| 1000 + i * 2000).collect();
    assert_eq!(r.user, expect);
    assert_eq!(r.state, SceneState::Done);
    assert_eq!(r.auto.len(), 30);
    for a in &r.auto {
        assert!(
            r.user.iter().all(|u| (u - a).abs() > 100),
            "auto {a} overlaps a user cut"
        );
    }
}

#[test]
fn t_scene_14_edits_never_touch_revision_or_the_search_outbox() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    add_video(&db.conn, 2, Some(60_000), Some(25.0));
    let snapshot = |c: &rusqlite::Connection| {
        (
            count(c, "SELECT SUM(revision) FROM media_assets"),
            count(c, "SELECT COUNT(*) FROM search_outbox"),
            count(c, "SELECT COALESCE(SUM(updated_at), 0) FROM media_assets"),
        )
    };
    let before = snapshot(&db.conn);

    // Act: 쓰기 경로 전부
    let opts = RequestOptions {
        redetect: true,
        restore_suppressed: true,
        sensitivity: Some(mam_db::SceneSensitivity::High),
    };
    request_tx(&db.conn, &[1, 2], &opts, 45, NOW).unwrap();
    scene_cuts::backfill_batch_tx(&db.conn, 0, 1000, NOW).unwrap();
    scene_cuts::requeue_pending_tx(&db.conn, NOW).unwrap();
    replace_auto_tx(&db.conn, 1, &[3000, 6000], "high", Some(25.0), NOW).unwrap();
    add_user_tx(&db.conn, 1, 4500, Some(25.0), None, NOW).unwrap();
    remove_tx(&db.conn, 1, 3000, CutSource::Auto, Some(25.0), NOW).unwrap();
    clear_auto_tx(&db.conn, 1, NOW).unwrap();
    set_outcome_tx(&db.conn, 2, SceneState::Failed, Some("io"), NOW).unwrap();

    // Assert
    assert_eq!(snapshot(&db.conn), before);
}
