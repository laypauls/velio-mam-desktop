//! 자동 추출 등록 훅 (`sceneDetectAuto`) — T-scene-35. 분류: 게이트 ON/OFF / Business rule / Validation / Concurrency(멱등).
//! 장면 실행기를 꽂지 않은 환경에서는 등록된 작업이 대기 상태로 남아 등록 결과(행·우선순위)를 그대로 볼 수 있다.

mod common;
mod scene_support;

use mam_core::jobs::{JobKind, NewJob};
use mam_db::scene_cuts::{SCENE_PRIORITY_BACKGROUND, SceneState};
use scene_support::{Fixture, Scene, cut_text};

fn quiet() -> Scene {
    Scene::Cuts(cut_text(&[(2.0, 60.0)]))
}

fn scene_rows(f: &Fixture) -> i64 {
    f.p.env.count("SELECT COUNT(*) FROM asset_scene_cuts")
}

#[test]
fn t_scene_35_off_by_default_registers_nothing() {
    // Arrange - 설정 없음(기본 끔)
    let f = Fixture::new(quiet(), false);
    // Act
    f.start();
    f.p.shutdown();
    // Assert
    assert_eq!(scene_rows(&f), 0);
    assert_eq!(f.scene_jobs(), 0);
    assert_eq!(f.runner.calls(), 0);
}

#[test]
fn t_scene_35_explicit_false_registers_nothing() {
    let f = Fixture::new(quiet(), false);
    f.set_setting("scene_detect_auto", "false");
    f.start();
    f.p.shutdown();
    assert_eq!(scene_rows(&f), 0);
    assert_eq!(f.scene_jobs(), 0);
}

#[test]
fn t_scene_35_invalid_setting_values_fall_back_to_off() {
    for bad in ["\"true\"", "1", "\"yes\"", "null", "[true]"] {
        let f = Fixture::new(quiet(), false);
        f.set_setting("scene_detect_auto", bad);
        f.start();
        f.p.shutdown();
        assert_eq!(scene_rows(&f), 0, "{bad}");
        assert_eq!(f.scene_jobs(), 0, "{bad}");
    }
}

#[test]
fn t_scene_35_on_registers_only_videos_at_background_priority() {
    // Arrange
    let f = Fixture::new(quiet(), false);
    f.set_setting("scene_detect_auto", "true");
    // Act
    f.start();
    f.p.shutdown();
    // Assert - 영상 1건만: 행 pending + 작업 60, 이미지·음성은 없음
    let vid = f.video();
    assert_eq!(scene_rows(&f), 1);
    assert_eq!(f.row(vid).unwrap().state, SceneState::Pending);
    assert_eq!(f.scene_jobs(), 1);
    assert_eq!(
        f.p.env.count(&format!(
            "SELECT priority FROM jobs WHERE kind = 'scene' AND asset_id = {vid} AND status = 'pending'"
        )),
        SCENE_PRIORITY_BACKGROUND
    );
    assert_eq!(f.runner.calls(), 0);
}

#[test]
fn t_scene_35_rerunning_meta_does_not_duplicate_the_registration() {
    // Arrange
    let f = Fixture::new(quiet(), false);
    f.set_setting("scene_detect_auto", "true");
    f.start();
    let vid = f.video();
    let jobs = f.p.env.ingest.jobs();
    // Act - 메타를 다시 실행한다 (행·작업이 이미 있다)
    jobs.enqueue(NewJob::for_asset(JobKind::Meta, vid)).unwrap();
    assert!(f.wait_except_scene(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    // Assert
    assert_eq!(scene_rows(&f), 1);
    assert_eq!(f.scene_jobs(), 1);
}

#[test]
fn t_scene_35_does_not_overwrite_an_existing_done_row() {
    // Arrange - 사용자 컷으로 이미 done 인 영상의 메타를 다시 돌려도 pending 으로 되돌리지 않는다
    let f = Fixture::new(quiet(), false);
    f.start();
    let vid = f.video();
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, 'done', 1)",
                [vid],
            )?;
            Ok(())
        })
        .unwrap();
    f.set_setting("scene_detect_auto", "true");
    // Act
    f.p.env
        .ingest
        .jobs()
        .enqueue(NewJob::for_asset(JobKind::Meta, vid))
        .unwrap();
    assert!(f.wait_except_scene());
    f.p.shutdown();
    // Assert
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
    assert_eq!(f.scene_jobs(), 0);
}

#[test]
fn t_scene_35_on_with_executor_extracts_in_the_background() {
    // Arrange
    let f = Fixture::new(Scene::Cuts(cut_text(&[(2.0, 60.0), (6.0, 40.0)])), true);
    f.set_setting("scene_detect_auto", "true");
    // Act
    f.start();
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    // Assert - 영상 1건만 검출되고 알림이 한 번 나간다
    let vid = f.video();
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
    assert_eq!(f.row(vid).unwrap().auto, vec![2000, 6000]);
    assert_eq!(f.runner.calls(), 1);
    assert_eq!(f.updated_ids(), vec![vid]);
    assert_eq!(scene_rows(&f), 1);
}
