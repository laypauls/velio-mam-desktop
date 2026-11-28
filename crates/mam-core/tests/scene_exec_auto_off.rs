//! 장면 자동 추출 옵션 OFF 뒤의 실행기·재등록 (T-scoff-10 ~ 14).
//! 분류: Business rule (자동 출처만 폐기·수동 보존) / 게이트 ON (옵션 켬이면 그대로 실행) / Concurrency (재등록 멱등).
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기. FFmpeg 는 `scdet` 호출만 대역이라 호출 횟수로 "실행 안 함"을 단언한다.

mod common;
mod scene_support;

use mam_db::Priority;
use mam_db::scene_cuts::{self, RequestOptions, SCENE_PRIORITY_USER, SceneState};
use scene_support::{Fixture, Scene, cut_text};

fn quiet() -> Scene {
    Scene::Cuts(cut_text(&[(2.0, 60.0)]))
}

/// 자동 등록 경로(`enqueue_auto_tx`)로 대기 행 + 작업을 만들고 깨운다.
fn register_auto(f: &Fixture, id: i64) {
    let now = f.p.env.clock_now();
    f.p.env
        .pool
        .write(Priority::High, move |c| {
            Ok(scene_cuts::enqueue_auto_tx(c, id, now))
        })
        .unwrap()
        .unwrap();
    f.p.env.ingest.jobs().notify();
}

fn insert_pending(f: &Fixture, id: i64, origin: &'static str) {
    f.p.env
        .pool
        .write(Priority::High, move |c| {
            c.execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at, origin)
                 VALUES (?1, 'pending', 1, ?2)",
                rusqlite::params![id, origin],
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_scoff_10_executor_drops_a_leftover_auto_row_when_the_option_is_off_without_running_ffmpeg() {
    // Arrange: 옵션 OFF(기본) 인데 자동 등록 행·작업이 남아 있다 (정리와 실행 사이 경합·구 데이터)
    let f = Fixture::new(quiet(), true);
    f.start();
    let vid = f.video();

    // Act
    register_auto(&f, vid);
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();

    // Assert
    assert_eq!(f.runner.calls(), 0);
    assert!(f.row(vid).is_none());
    assert!(f.updated_ids().is_empty());
}

#[test]
fn t_scoff_11_executor_runs_the_auto_row_when_the_option_is_on() {
    // Arrange: 게이트 ON
    let f = Fixture::new(quiet(), true);
    f.set_setting("scene_detect_auto", "true");
    f.start();
    let vid = f.video();

    // Act: 메타 완료 훅이 이미 등록했을 수 있다 - 행·작업 유무와 무관하게 끝까지 처리된다
    register_auto(&f, vid);
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();

    // Assert
    assert_eq!(f.runner.calls(), 1);
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
}

#[test]
fn t_scoff_12_manual_request_runs_even_when_the_option_is_off() {
    // Arrange: 옵션 OFF
    let f = Fixture::new(quiet(), true);
    f.start();
    let vid = f.video();

    // Act
    f.request(&[vid], RequestOptions::default(), SCENE_PRIORITY_USER);
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();

    // Assert
    assert_eq!(f.runner.calls(), 1);
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
}

#[test]
fn t_scoff_13_restart_requeue_skips_and_clears_auto_rows_when_off_but_requeues_manual() {
    // Arrange: 작업이 사라진 대기 행 - 자동 1, 수동 1, 구 행(NULL) 1 (실행기 없음)
    let f = Fixture::new(Scene::Block, false);
    f.p.put("lib/v2.mp4", b"second video");
    f.p.put("lib/v3.mp4", b"third video");
    f.start();
    let ids: Vec<i64> = ["v.mp4", "v2.mp4", "v3.mp4"]
        .iter()
        .map(|n| f.p.asset_of(n))
        .collect();
    insert_pending(&f, ids[0], "auto");
    insert_pending(&f, ids[1], "manual");
    let legacy = ids[2];
    f.p.env
        .pool
        .write(Priority::High, move |c| {
            c.execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, 'pending', 1)",
                [legacy],
            )?;
            Ok(())
        })
        .unwrap();

    // Act
    let n = mam_core::scene::requeue_pending(&f.p.env.pool, 5).unwrap();
    let again = mam_core::scene::requeue_pending(&f.p.env.pool, 6).unwrap();

    // Assert: 자동분은 지워지고 재등록되지 않는다, 수동·구 행은 등록
    assert_eq!(n, 2);
    assert_eq!(again, 0, "멱등");
    assert!(f.row(ids[0]).is_none());
    assert!(f.row(ids[1]).is_some());
    assert_eq!(f.scene_jobs(), 2);
    f.p.shutdown();
}

#[test]
fn t_scoff_14_restart_requeue_keeps_auto_rows_when_the_option_is_on() {
    // Arrange: 게이트 ON
    let f = Fixture::new(Scene::Block, false);
    f.set_setting("scene_detect_auto", "true");
    f.start();
    let vid = f.video();
    // 메타 완료 훅이 이미 행·작업을 만들었다 - 작업만 지워 "사라진 작업" 상태를 만든다
    f.p.env
        .pool
        .write(Priority::High, |c| {
            c.execute("DELETE FROM jobs WHERE kind = 'scene'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(f.row(vid).unwrap().state, SceneState::Pending);

    // Act
    let n = mam_core::scene::requeue_pending(&f.p.env.pool, 5).unwrap();

    // Assert
    assert_eq!(n, 1);
    assert_eq!(f.scene_jobs(), 1);
    assert!(f.row(vid).is_some());
    f.p.shutdown();
}
