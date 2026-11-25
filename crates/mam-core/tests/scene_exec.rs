//! 장면 검출 실행기 (`SceneExecutor`) — T-scene-32 ~ 36.
//! 분류: Happy / Business rule (32, 36) / Concurrency·Time (33) / External (34).
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기(스레드). FFmpeg 는 `scdet` 호출만 대역으로 정한다.

mod common;
mod scene_support;

use std::time::Duration;

use common::env::{snapshot, wait_until};
use mam_core::jobs::JobKind;
use mam_db::SceneSensitivity;
use mam_db::scene_cuts::{
    RequestOptions, SCENE_PRIORITY_BACKGROUND, SCENE_PRIORITY_USER, SceneState,
};
use mam_media::MediaError;
use scene_support::{Fixture, Scene, cut_text};

fn opts() -> RequestOptions {
    RequestOptions::default()
}

/// (revision, outbox 이벤트 수)
fn index_state(f: &Fixture, asset_id: i64) -> (i64, i64) {
    (
        f.p.revision(asset_id),
        f.p.env.count(&format!(
            "SELECT COUNT(*) FROM search_outbox WHERE asset_id = {asset_id}"
        )),
    )
}

/// 길이 10초(probe_json) 영상: 0.4s(시작 0.5s 안 → 버림), 1.0s·1.5s(1초 안 연쇄 → 점수 높은 쪽), 5s, 9.8s(끝 0.5s 안 → 버림)
fn typical() -> Scene {
    Scene::Cuts(cut_text(&[
        (0.4, 90.0),
        (1.0, 80.0),
        (1.5, 30.0),
        (5.0, 50.0),
        (9.8, 70.0),
    ]))
}

#[test]
fn t_scene_34_success_stores_selected_cuts_once_and_never_touches_the_originals() {
    // Arrange
    let f = Fixture::new(typical(), true);
    f.start();
    let vid = f.video();
    let before = snapshot(&f.p.env.vol);
    let index_before = index_state(&f, vid);
    // Act
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    // Assert - select_auto 규칙 적용 결과, 상태·지문·알림 1회
    let row = f.row(vid).unwrap();
    assert_eq!(row.state, SceneState::Done);
    assert_eq!(row.auto, vec![1000, 5000]);
    assert_eq!(row.sensitivity.as_deref(), Some("medium"));
    assert_eq!(
        row.detector_version,
        Some(mam_db::scene_cuts::SCENE_DETECTOR_VERSION)
    );
    assert!(row.error.is_none() && row.requested_sensitivity.is_none());
    assert_eq!(f.updated_ids(), vec![vid]);
    assert_eq!(f.runner.calls(), 1);
    assert_eq!(f.scene_jobs(), 0);
    // 원본 보호: 내용·크기·수정 시각 불변
    assert_eq!(snapshot(&f.p.env.vol), before);
    // 검색 색인 대상이 아니다 - revision·outbox 불변
    assert_eq!(index_state(&f, vid), index_before);
}

#[test]
fn t_scene_34_non_video_assets_are_unsupported_without_running_ffmpeg() {
    // Arrange - 음성에 요청을 강제로 올린다 (request_tx 는 영상만 받으므로 행·작업을 직접 만든다)
    let f = Fixture::new(typical(), true);
    f.start();
    let aud = f.p.asset_of("s.wav");
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, 'pending', 1)",
                [aud],
            )?;
            c.execute(
                "INSERT INTO jobs (kind, asset_id, priority, created_at, updated_at) VALUES ('scene', ?1, 60, 1, 1)",
                [aud],
            )?;
            Ok(())
        })
        .unwrap();
    f.p.env.ingest.jobs().notify();
    // Act
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    // Assert
    assert_eq!(f.row(aud).unwrap().state, SceneState::Unsupported);
    assert_eq!(f.runner.calls(), 0);
    assert_eq!(f.updated_ids(), vec![aud]);
}

#[test]
fn t_scene_34_tool_errors_map_to_fixed_failure_codes_and_keep_existing_cuts() {
    // (모드, 기대 상태, 기대 오류 코드, 호출 횟수)
    type Case = (Scene, SceneState, Option<&'static str>, usize);
    let cases: Vec<Case> = vec![
        (
            Scene::Err(|| MediaError::ToolNotFound {
                tool: "ffmpeg",
                searched: "x".into(),
            }),
            SceneState::Failed,
            Some("tool_not_found"),
            1,
        ),
        (
            Scene::Exit(1, "Invalid data found when processing input"),
            SceneState::Failed,
            Some("tool_failed"),
            1,
        ),
        (
            Scene::Err(|| MediaError::OutputTooLarge {
                tool: "ffmpeg",
                limit: 1,
            }),
            SceneState::Failed,
            Some("output_too_large"),
            1,
        ),
        // 영상 스트림 없음 -> 미지원 (오류 코드 없음)
        (
            Scene::Exit(127, "Stream map '' matches no streams."),
            SceneState::Unsupported,
            None,
            1,
        ),
        // 일시 오류는 백오프로 MAX_ATTEMPTS(5) 번 시도한 뒤 실패로 기록한다 (수동 시계를 당겨 결정적으로)
        (
            Scene::Err(|| MediaError::Timeout {
                tool: "ffmpeg",
                timeout: Duration::from_secs(1),
            }),
            SceneState::Failed,
            Some("timeout"),
            5,
        ),
        (
            Scene::Err(|| MediaError::Io {
                path: "x".into(),
                source: std::io::Error::other("share violation"),
            }),
            SceneState::Failed,
            Some("io"),
            5,
        ),
    ];
    for (mode, state, code, calls) in cases {
        // Arrange - 이미 사용자 컷과 자동 컷이 있는 영상
        let f = Fixture::new(typical(), true);
        f.start();
        let vid = f.video();
        f.request(&[vid], opts(), SCENE_PRIORITY_USER);
        assert!(f.p.wait_idle());
        let kept = f.row(vid).unwrap().auto;
        assert_eq!(kept, vec![1000, 5000]);
        let updates_before = f.updated_ids().len();
        f.runner.set(mode);
        // Act - 다시 추출
        f.request(
            &[vid],
            RequestOptions {
                redetect: true,
                ..opts()
            },
            SCENE_PRIORITY_USER,
        );
        assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
        f.p.shutdown();
        // Assert
        let row = f.row(vid).unwrap();
        assert_eq!((row.state, row.error.as_deref()), (state, code));
        assert_eq!(row.auto, kept, "failed run must keep the previous cuts");
        assert_eq!(f.runner.calls() - 1, calls);
        assert_eq!(f.updated_ids().len(), updates_before + 1, "one notify");
        assert_eq!(
            f.p.env
                .count("SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND status = 'failed'"),
            0,
            "the job ends as Done after the outcome is recorded"
        );
    }
}

#[test]
fn t_scene_34_requested_sensitivity_wins_over_the_setting() {
    // Arrange - 설정은 low(20), 요청은 high(5), 요청 없는 자산은 설정을 따른다
    let f = Fixture::new(typical(), true);
    f.set_setting("scene_detect_sensitivity", "\"low\"");
    f.start();
    let vid = f.video();
    // Act
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    assert!(f.p.wait_idle());
    f.request(
        &[vid],
        RequestOptions {
            redetect: true,
            sensitivity: Some(SceneSensitivity::High),
            ..opts()
        },
        SCENE_PRIORITY_USER,
    );
    assert!(f.p.wait_idle());
    f.p.shutdown();
    // Assert
    let filters = f.runner.filters.lock().unwrap().clone();
    assert!(filters[0].contains("scdet=threshold=20,"), "{filters:?}");
    assert!(filters[1].contains("scdet=threshold=5,"), "{filters:?}");
    assert_eq!(f.row(vid).unwrap().sensitivity.as_deref(), Some("high"));
}

#[test]
fn t_scene_32_offline_ends_the_job_keeps_pending_and_requeues_when_the_volume_returns() {
    // Arrange
    let f = Fixture::new(typical(), true);
    f.start();
    let vid = f.video();
    f.p.env.volumes.set_connected("vol-a", false);
    assert!(
        wait_until(Duration::from_secs(30), || f.p.env.count(
            "SELECT COUNT(*) FROM volumes WHERE availability = 'online'"
        ) == 0),
        "volume never went offline"
    );
    // Act - 오프라인 상태에서 요청
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    // Assert - 작업은 끝났지만 행은 pending 이고 FFmpeg 는 돌지 않았다 (삭제가 아니다)
    assert_eq!(f.row(vid).unwrap().state, SceneState::Pending);
    assert_eq!(f.runner.calls(), 0);
    assert_eq!(f.scene_jobs(), 0);
    // Act - 볼륨 복귀 -> 주기 점검이 pending 행을 재등록
    f.p.env.volumes.set_connected("vol-a", true);
    assert!(
        wait_until(Duration::from_secs(60), || {
            f.row(vid).is_some_and(|r| r.state == SceneState::Done)
        }),
        "never re-queued: {}",
        f.p.jobs_dump()
    );
    f.p.shutdown();
    // Assert
    assert_eq!(f.row(vid).unwrap().auto, vec![1000, 5000]);
    assert_eq!(f.runner.calls(), 1);
}

#[test]
fn t_scene_32_inactive_asset_is_dropped_without_touching_the_row() {
    // Arrange
    let f = Fixture::new(typical(), true);
    f.start();
    let vid = f.video();
    // 작업이 시작되기 전에 자산을 바꾸기 위해 종류를 멈춰 둔다
    let jobs = f.p.env.ingest.jobs();
    jobs.pause(Some(JobKind::Scene)).unwrap();
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "UPDATE media_assets SET status = 'superseded' WHERE asset_id = ?1",
                [vid],
            )?;
            Ok(())
        })
        .unwrap();
    // Act
    jobs.resume(Some(JobKind::Scene)).unwrap();
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    // Assert
    assert_eq!(f.runner.calls(), 0);
    assert_eq!(f.row(vid).unwrap().state, SceneState::Pending);
    assert!(f.updated_ids().is_empty());
}

#[test]
fn t_scene_32_pending_metadata_defers_the_job_without_burning_attempts() {
    // Arrange - 메타 파생물 행을 지워 "아직 메타 전" 상태로 만든다
    let f = Fixture::new(typical(), true);
    f.start();
    let vid = f.video();
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "DELETE FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'meta'",
                [vid],
            )?;
            Ok(())
        })
        .unwrap();
    let t0 = f.p.env.clock_now();
    // Act
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    let deferred = wait_until(Duration::from_secs(30), || {
        f.p.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND not_before > 0")
            == 1
            && f.p
                .env
                .count("SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND status = 'pending'")
                == 1
    });
    // Assert - 60초 뒤로 미뤄지고 시도 횟수는 그대로, FFmpeg 미실행
    assert!(deferred, "{}", f.p.jobs_dump());
    assert_eq!(
        f.p.env
            .count("SELECT not_before FROM jobs WHERE kind = 'scene'"),
        t0 + 60_000
    );
    assert_eq!(
        f.p.env
            .count("SELECT attempts FROM jobs WHERE kind = 'scene'"),
        0
    );
    assert_eq!(f.runner.calls(), 0);
    // Act - 메타가 끝난 것으로 하고 시계를 60초 당기면 실행된다
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) VALUES (?1, 'meta', 'done', 1)",
                [vid],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
}

#[test]
fn t_scene_33_pause_kills_ffmpeg_yields_without_attempts_and_resume_reruns_from_the_start() {
    // Arrange - 취소될 때까지 도는 FFmpeg
    let f = Fixture::new(Scene::Block, true);
    f.start();
    let vid = f.video();
    let before = snapshot(&f.p.env.vol);
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    assert!(
        wait_until(Duration::from_secs(30), || f.runner.calls() == 1),
        "ffmpeg never started"
    );
    // Act - 실행 중 일시정지
    f.p.env.ingest.jobs().pause(Some(JobKind::Scene)).unwrap();
    // Assert - 프로세스를 죽이고 양보: 대기로 복귀, 시도 횟수 불변, 결과·알림 없음
    assert!(
        wait_until(Duration::from_secs(30), || f.runner.killed() == 1
            && f.p.env.count(
                "SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND status = 'pending'"
            ) == 1),
        "not yielded: {}",
        f.p.jobs_dump()
    );
    assert_eq!(
        f.p.env
            .count("SELECT attempts FROM jobs WHERE kind = 'scene'"),
        0
    );
    assert_eq!(f.row(vid).unwrap().state, SceneState::Pending);
    assert!(f.row(vid).unwrap().auto.is_empty());
    assert!(f.updated_ids().is_empty());
    assert_eq!(f.runner.calls(), 1, "paused kind must not restart");
    // Act - 재개: 처음부터 다시 실행해 완료
    f.runner.set(typical());
    f.p.env.ingest.jobs().resume(None).unwrap();
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    // Assert
    assert_eq!(f.runner.calls(), 2);
    assert_eq!(f.row(vid).unwrap().state, SceneState::Done);
    assert_eq!(f.row(vid).unwrap().auto, vec![1000, 5000]);
    assert_eq!(f.updated_ids(), vec![vid]);
    assert_eq!(snapshot(&f.p.env.vol), before);
}

#[test]
fn t_scene_36_result_is_discarded_when_the_asset_changed_during_detection() {
    // Arrange - 검출 중(Block)에 스캔이 자산을 갱신한 것과 같은 변화를 DB 에 만든다
    let f = Fixture::new(Scene::Block, true);
    f.start();
    let vid = f.video();
    f.request(&[vid], opts(), SCENE_PRIORITY_USER);
    assert!(wait_until(Duration::from_secs(30), || f.runner.calls() == 1));
    f.p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "UPDATE media_assets SET file_size = file_size + 1 WHERE asset_id = ?1",
                [vid],
            )?;
            Ok(())
        })
        .unwrap();
    // Act - 검출이 끝난다
    f.runner.set(typical());
    assert!(f.p.wait_idle(), "{}", f.p.jobs_dump());
    f.p.shutdown();
    // Assert - 결과 폐기: 컷·알림 없음, 행은 pending 그대로, 작업은 끝남
    let row = f.row(vid).unwrap();
    assert_eq!(row.state, SceneState::Pending);
    assert!(row.auto.is_empty());
    assert!(f.updated_ids().is_empty());
    assert_eq!(f.scene_jobs(), 0);
}

#[test]
fn t_scene_priority_constants_match_the_job_kind_defaults() {
    // 사용자 요청 45 는 메타·썸네일(20)·AI(50) 사이, 백그라운드 60 은 AI 뒤
    assert_eq!(JobKind::Scene.default_priority(), SCENE_PRIORITY_BACKGROUND);
    assert_eq!(SCENE_PRIORITY_USER, 45);
    assert!(SCENE_PRIORITY_USER < JobKind::Ai.default_priority());
}
