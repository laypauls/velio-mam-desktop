//! 장면전환점 사용자 편집 (T-scene-08·09 의 UserData 경계) — 실제 SQLite(V15 적용) 위의 get/add/remove/clear_auto.
//!
//! 분류: Happy (계약 모양·병합 정렬) / Validation (영상 아님·범위·상한) / Business rule (숨김 기억·허용 오차·
//! revision·outbox 불변) / Concurrency (동시 추가)

use std::sync::Arc;

use mam_core::SharedClock;
use mam_core::scan::IngestError;
use mam_core::userdata::{CutSource, SceneCut, SceneFailure, SceneStateView, UserData};
use mam_db::scene_cuts::{self, MAX_USER_CUTS};
use mam_db::{DbPool, SceneSensitivity, SystemClock, migrate, open};
use rusqlite::{Connection, params};
use tempfile::TempDir;

struct Fx {
    dir: TempDir,
    ud: Arc<UserData>,
}

impl Fx {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db, 2).unwrap());
        let clock: SharedClock = Arc::new(SystemClock);
        Self {
            dir,
            ud: Arc::new(UserData::new(pool, clock)),
        }
    }

    fn raw(&self) -> Connection {
        open(&self.dir.path().join("mam.sqlite")).unwrap()
    }

    fn asset(&self, id: i64, media: &str, status: &str, duration: Option<i64>, fps: Option<f64>) {
        let c = self.raw();
        c.execute(
            "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 10, 0, 0, 0)",
            params![id, media, status],
        )
        .unwrap();
        if duration.is_some() || fps.is_some() {
            c.execute(
                "INSERT INTO media_metadata (asset_id, duration_ms, fps) VALUES (?1, ?2, ?3)",
                params![id, duration, fps],
            )
            .unwrap();
        }
    }

    /// 길이 60초, 25fps (허용 오차 40ms) 영상
    fn video(&self, id: i64) {
        self.asset(id, "video", "active", Some(60_000), Some(25.0));
    }

    fn seed_auto(&self, id: i64, cuts: &[i64]) {
        scene_cuts::replace_auto_tx(&self.raw(), id, cuts, "medium", Some(25.0), 1).unwrap();
    }

    fn count(&self, sql: &str) -> i64 {
        self.raw().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn cut(ms: i64, source: CutSource) -> SceneCut {
    SceneCut { ms, source }
}

#[test]
fn t_scene_08_get_reports_none_for_a_video_without_a_row_and_not_found_for_unknown_assets() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.asset(2, "image", "active", None, None);

    // Act
    let none = fx.ud.scene_cuts_get(1).unwrap();
    let image = fx.ud.scene_cuts_get(2).unwrap();
    let missing = fx.ud.scene_cuts_get(99).unwrap_err();

    // Assert
    assert_eq!(none.state, SceneStateView::None);
    assert!(none.cuts.is_empty() && none.sensitivity.is_none() && none.updated_at.is_none());
    assert_eq!(
        image.state,
        SceneStateView::None,
        "조회는 영상 여부를 따지지 않는다"
    );
    assert!(matches!(missing, IngestError::NotFound));
}

#[test]
fn t_scene_08_edits_reject_non_video_missing_and_inactive_assets_without_writing() {
    // Arrange
    let fx = Fx::new();
    fx.asset(1, "image", "active", None, None);
    fx.asset(2, "audio", "active", Some(1000), None);
    fx.asset(3, "video", "superseded", Some(60_000), Some(25.0));
    fx.asset(4, "video", "deleted", Some(60_000), Some(25.0));

    // Act / Assert
    for id in [1, 2] {
        assert!(
            matches!(
                fx.ud.scene_cut_add(id, 100),
                Err(IngestError::InvalidInput("mediaType"))
            ),
            "{id}"
        );
        assert!(matches!(
            fx.ud.scene_cut_remove(id, 100, CutSource::User),
            Err(IngestError::InvalidInput("mediaType"))
        ));
        assert!(matches!(
            fx.ud.scene_cuts_clear_auto(id),
            Err(IngestError::InvalidInput("mediaType"))
        ));
    }
    for id in [3, 4, 99] {
        assert!(
            matches!(fx.ud.scene_cut_add(id, 100), Err(IngestError::NotFound)),
            "{id}"
        );
        assert!(matches!(
            fx.ud.scene_cut_remove(id, 100, CutSource::Auto),
            Err(IngestError::NotFound)
        ));
        assert!(matches!(
            fx.ud.scene_cuts_clear_auto(id),
            Err(IngestError::NotFound)
        ));
    }
    assert_eq!(fx.count("SELECT COUNT(*) FROM asset_scene_cuts"), 0);
}

#[test]
fn t_scene_08_add_validates_ms_against_duration_and_reports_the_merged_contract_shape() {
    // Arrange: 길이 60,000ms
    let fx = Fx::new();
    fx.video(1);
    fx.seed_auto(1, &[10_000, 30_000]);

    // Act
    for bad in [-1, 60_001, i64::MAX] {
        assert!(
            matches!(
                fx.ud.scene_cut_add(1, bad),
                Err(IngestError::InvalidInput("ms"))
            ),
            "{bad}"
        );
    }
    let out = fx.ud.scene_cut_add(1, 20_000).unwrap();
    let edge = fx.ud.scene_cut_add(1, 60_000).unwrap();

    // Assert: ms 오름차순 병합 + 출처 구분, 추출 지문 유지
    assert_eq!(
        out.cuts,
        vec![
            cut(10_000, CutSource::Auto),
            cut(20_000, CutSource::User),
            cut(30_000, CutSource::Auto)
        ]
    );
    assert_eq!((out.asset_id, out.state), (1, SceneStateView::Done));
    assert_eq!(out.sensitivity, Some(SceneSensitivity::Medium));
    assert_eq!(edge.cuts.last(), Some(&cut(60_000, CutSource::User)));
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["cuts"][1],
        serde_json::json!({"ms": 20_000, "source": "user"})
    );
    assert_eq!(json["suppressedCount"], 0);
    assert_eq!(json["state"], "done");
}

#[test]
fn t_scene_08_add_within_one_frame_of_an_existing_cut_is_a_noop_and_cap_is_enforced() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.seed_auto(1, &[10_000]);
    fx.ud.scene_cut_add(1, 20_000).unwrap();
    let before = fx.ud.scene_cuts_get(1).unwrap();

    // Act: 25fps → ±40ms
    let near_auto = fx.ud.scene_cut_add(1, 10_040).unwrap();
    let near_user = fx.ud.scene_cut_add(1, 19_960).unwrap();

    // Assert: 변화 없음 (updated_at 포함)
    assert_eq!(near_auto, before);
    assert_eq!(near_user, before);

    // 상한: 사용자 컷을 가득 채우면 다음 추가는 거부, 기존 위치는 여전히 no-op
    fx.asset(2, "video", "active", None, None);
    {
        let c = fx.raw();
        let tx = c.unchecked_transaction().unwrap();
        for i in 0..MAX_USER_CUTS as i64 {
            scene_cuts::add_user_tx(&tx, 2, i * 1000, None, None, 1).unwrap();
        }
        tx.commit().unwrap();
    }
    assert!(matches!(
        fx.ud.scene_cut_add(2, 9_000_000),
        Err(IngestError::InvalidInput("cuts"))
    ));
    assert_eq!(
        fx.ud.scene_cut_add(2, 1000).unwrap().cuts.len(),
        MAX_USER_CUTS
    );
}

#[test]
fn t_scene_09_remove_auto_is_remembered_user_is_forgotten_and_both_are_idempotent() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.seed_auto(1, &[10_000, 30_000]);
    fx.ud.scene_cut_add(1, 20_000).unwrap();

    // Act
    let after_auto = fx.ud.scene_cut_remove(1, 10_030, CutSource::Auto).unwrap();
    let again = fx.ud.scene_cut_remove(1, 10_030, CutSource::Auto).unwrap();
    let wrong_source = fx.ud.scene_cut_remove(1, 20_000, CutSource::Auto).unwrap();
    let after_user = fx.ud.scene_cut_remove(1, 20_010, CutSource::User).unwrap();

    // Assert
    assert_eq!(
        after_auto.cuts,
        vec![cut(20_000, CutSource::User), cut(30_000, CutSource::Auto)]
    );
    assert_eq!(after_auto.suppressed_count, 1);
    assert_eq!(again.cuts, after_auto.cuts, "멱등");
    assert_eq!(wrong_source.cuts, after_auto.cuts, "출처가 다르면 no-op");
    assert_eq!(after_user.cuts, vec![cut(30_000, CutSource::Auto)]);
    assert_eq!(
        after_user.suppressed_count, 1,
        "사용자 컷은 기억하지 않는다"
    );
    // 행이 없는 영상에서의 제거는 no-op 이고 행을 만들지 않는다
    fx.video(2);
    let none = fx.ud.scene_cut_remove(2, 5000, CutSource::Auto).unwrap();
    assert_eq!(none.state, SceneStateView::None);
}

#[test]
fn t_scene_09_clear_auto_keeps_user_cuts_and_a_later_detection_does_not_revive_the_hidden_ones() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.seed_auto(1, &[10_000, 30_000, 50_000]);
    fx.ud.scene_cut_add(1, 20_000).unwrap();

    // Act
    let cleared = fx.ud.scene_cuts_clear_auto(1).unwrap();
    let again = fx.ud.scene_cuts_clear_auto(1).unwrap();
    fx.seed_auto(1, &[10_000, 30_000, 50_000, 55_000]);
    let redetected = fx.ud.scene_cuts_get(1).unwrap();

    // Assert
    assert_eq!(cleared.cuts, vec![cut(20_000, CutSource::User)]);
    assert_eq!(cleared.suppressed_count, 3);
    assert_eq!(again.cuts, cleared.cuts);
    assert_eq!(
        redetected.cuts,
        vec![cut(20_000, CutSource::User), cut(55_000, CutSource::Auto)]
    );
    assert_eq!(redetected.suppressed_count, 3);
}

#[test]
fn t_scene_09_failure_and_unknown_stored_values_map_to_contract_codes() {
    // Arrange: 저장 오류 코드·민감도가 앱이 모르는 값이어도 읽기는 실패하지 않는다
    let fx = Fx::new();
    fx.video(1);
    fx.video(2);
    fx.raw()
        .execute_batch(
            "INSERT INTO asset_scene_cuts (asset_id, state, error, sensitivity, updated_at) VALUES (1, 'failed', 'output_too_large', 'ultra', 7);
             INSERT INTO asset_scene_cuts (asset_id, state, error, updated_at) VALUES (2, 'failed', 'from_the_future', 8);",
        )
        .unwrap();

    // Act
    let a = fx.ud.scene_cuts_get(1).unwrap();
    let b = fx.ud.scene_cuts_get(2).unwrap();

    // Assert
    assert_eq!(
        (a.state, a.error, a.sensitivity, a.updated_at),
        (
            SceneStateView::Failed,
            Some(SceneFailure::OutputTooLarge),
            None,
            Some(7)
        )
    );
    assert_eq!(b.error, Some(SceneFailure::Other));
    assert_eq!(
        serde_json::to_value(&a).unwrap()["error"],
        "output_too_large"
    );
}

#[test]
fn t_scene_14_user_edits_never_touch_revision_or_the_search_outbox() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.seed_auto(1, &[10_000]);
    let snap = |fx: &Fx| {
        (
            fx.count("SELECT revision FROM media_assets WHERE asset_id = 1"),
            fx.count("SELECT COUNT(*) FROM search_outbox"),
        )
    };
    let before = snap(&fx);

    // Act
    fx.ud.scene_cut_add(1, 20_000).unwrap();
    fx.ud.scene_cut_remove(1, 10_000, CutSource::Auto).unwrap();
    fx.ud.scene_cuts_clear_auto(1).unwrap();

    // Assert
    assert_eq!(snap(&fx), before);
}

#[test]
fn t_scene_10_concurrent_adds_from_many_threads_all_land() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);

    // Act: 서로 다른 위치 16 곳을 동시에 추가
    let handles: Vec<_> = (0..16i64)
        .map(|i| {
            let ud = Arc::clone(&fx.ud);
            std::thread::spawn(move || ud.scene_cut_add(1, 1000 + i * 3000).unwrap())
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Assert
    let got = fx.ud.scene_cuts_get(1).unwrap();
    let ms: Vec<i64> = got.cuts.iter().map(|c| c.ms).collect();
    assert_eq!(ms, (0..16).map(|i| 1000 + i * 3000).collect::<Vec<_>>());
    assert!(got.cuts.iter().all(|c| c.source == CutSource::User));
}
