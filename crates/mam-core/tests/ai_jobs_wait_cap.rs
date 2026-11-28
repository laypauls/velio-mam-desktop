//! T-aiwait-02 ~ 05 - 파생물(프레임 스트립) 대기 상한 30분: 첫 대기부터 상한을 넘기면 `thumbnail_missing` 재시도 실패.
//! 분류: Time (주입 시계로 30분 경계) / Boundary (상한 직전·직후) / Business rule (시도·이력 소모는 상한 뒤부터) /
//! Concurrency (실패 후 새 대기 창).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::AiSettingsSource;
use mam_core::jobs::JobOutcome;
use mam_db::Clock;

const MIN: i64 = 60_000;

/// 스트립이 아직 없는 신규 영상 + 자동 분석 등록 (외부 분석 describe 단계 + 임베딩).
fn waiting_video(e: &AiEnv) -> i64 {
    let id = e.add_asset("video");
    e.set_strip(id, None, false);
    e.set(|s| {
        s.auto_analyze_new = true;
        s.stt_enabled = false;
    });
    let st = e.settings.snapshot();
    assert_eq!(e.sched.enqueue_new_asset(id, &st).unwrap().queued, 1);
    id
}

fn wait_since(e: &AiEnv, id: i64, kind: &'static str) -> Option<i64> {
    e.env.read(|c| {
        c.query_row(
            "SELECT wait_since FROM ai_task_state WHERE asset_id = ?1 AND kind = ?2",
            (id, kind),
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn attempts_spent(e: &AiEnv) -> i64 {
    e.env
        .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'")
}

#[test]
fn t_aiwait_02_waiting_past_the_cap_fails_with_thumbnail_missing_and_records_history() {
    // Arrange
    let e = AiEnv::new();
    let id = waiting_video(&e);
    let first = e.env.clock.now_ms();

    // Act + Assert: 첫 대기 - 시각 기록, 시도 없음
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    assert_eq!(wait_since(&e, id, "describe"), Some(first));
    // 상한 직전(29분) - 계속 기다린다. 첫 대기 시각은 그대로
    e.env.clock.advance(29 * MIN);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    assert_eq!(wait_since(&e, id, "describe"), Some(first));
    assert_eq!(attempts_spent(&e), 0);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM analysis_runs"), 0);

    // 상한 직후 (첫 대기 + 30분 1초)
    e.env.clock.advance(MIN + 1_000);
    let err = e.run_next(&|| false).unwrap().unwrap_err();

    // Assert: 재시도 가능 실패, 단계는 queued + thumbnail_missing, 이력 1건, 새 창을 위해 대기 시각 초기화
    assert!(err.retryable);
    assert_eq!(
        e.step(id, "describe"),
        Some(("queued".into(), Some("thumbnail_missing".into())))
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE error_code = 'thumbnail_missing'"),
        5,
        "외부 4종 + 임베딩 단계마다 실패 이력"
    );
    assert_eq!(wait_since(&e, id, "describe"), None);
    assert!(e.ext.calls.lock().unwrap().is_empty(), "전송 없음");
}

#[test]
fn t_aiwait_03_cap_boundary_one_ms_before_keeps_waiting_and_exactly_at_the_cap_fails() {
    for (elapsed, expect_fail) in [(30 * MIN - 1, false), (30 * MIN, true)] {
        // Arrange: 첫 대기를 기록한 뒤 `elapsed` 가 지난 시점
        let e = AiEnv::new();
        let id = waiting_video(&e);
        assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
        e.env.clock.advance(elapsed);

        // Act
        let out = e.run_next(&|| false).unwrap();

        // Assert
        assert_eq!(out.is_err(), expect_fail, "elapsed={elapsed}");
        let code = e.step(id, "describe").and_then(|s| s.1);
        assert_eq!(
            code.as_deref(),
            expect_fail.then_some("thumbnail_missing"),
            "elapsed={elapsed}"
        );
    }
}

#[test]
fn t_aiwait_04_after_the_failure_the_next_wait_starts_a_fresh_window() {
    // Arrange: 상한 초과로 한 번 실패
    let e = AiEnv::new();
    let id = waiting_video(&e);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    e.env.clock.advance(31 * MIN);
    assert!(e.run_next(&|| false).unwrap().is_err());

    // Act: 백오프를 넘겨 다시 실행 - 여전히 파생물이 없다
    e.env.clock.advance(10 * MIN);
    let out = e.run_next(&|| false);

    // Assert: 곧바로 또 실패하지 않고 새로 기다린다 (새 첫 대기 시각)
    assert_eq!(out, Some(Ok(JobOutcome::Yielded)));
    assert_eq!(
        wait_since(&e, id, "describe"),
        Some(e.env.clock.now_ms()),
        "새 창"
    );
}

#[test]
fn t_aiwait_05_derivative_ready_before_the_cap_runs_and_clears_the_wait_marker() {
    // Arrange: 20분 기다린 뒤 스트립이 완성된다
    let e = AiEnv::new();
    let id = waiting_video(&e);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    e.env.clock.advance(20 * MIN);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    e.set_strip(id, Some("done"), true);
    e.env.clock.advance(MIN);

    // Act
    let out = e.run_next(&|| false);

    // Assert
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(wait_since(&e, id, "describe"), None);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE error_code IS NOT NULL"),
        0
    );
}
