//! 구독 한도(`subscription_limit`) 는 시도 횟수를 쓰지 않고 작업을 한도 해제 시각까지 미룬다.
//! 분류: Business rule / Time / Boundary(알 수 없는 해제 시각, 시도 한도 초과 횟수) / Concurrency(재실행)
//! 또 요청이 나가지 않은 CLI 실패는 사용량 행을 남기지 않고, 실제 과금 방식이 있으면 그것을 기록한다.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use common::env::T0;
use mam_core::ai_jobs::StepFailure;
use mam_core::jobs::{JobOutcome, MAX_ATTEMPTS};

fn limit(retry_after: Option<u64>) -> StepFailure {
    StepFailure::new("subscription_limit", "구독 한도").with_retry_after(retry_after)
}

fn attempts(e: &AiEnv) -> i64 {
    e.env.count("SELECT attempts FROM jobs")
}

fn not_before(e: &AiEnv) -> i64 {
    e.env.count("SELECT not_before FROM jobs")
}

#[test]
fn t_aij_sub_01_limit_defers_the_job_without_counting_an_attempt() {
    // Arrange
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(limit(Some(86_400))));

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: 실패가 아니라 양보 - 시도 0, 해제 시각까지 대기, 단계는 queued(+코드), 실패 이력 없음
    assert_eq!(out, Ok(JobOutcome::Yielded));
    assert_eq!(attempts(&e), 0);
    assert_eq!(not_before(&e), T0 + 86_400_000);
    assert_eq!(
        e.step(id, "describe"),
        Some(("queued".into(), Some("subscription_limit".into())))
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM analysis_runs"), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        0
    );
    assert!(
        e.run_next(&|| false).is_none(),
        "해제 시각 전에는 다시 꺼내지지 않는다"
    );
}

#[test]
fn t_aij_sub_02_unknown_reset_time_waits_one_hour() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext.script.lock().unwrap().push_back(Err(limit(None)));

    e.run_next(&|| false).unwrap().unwrap();

    assert_eq!(not_before(&e), T0 + 3_600_000);
}

#[test]
fn t_aij_sub_03_repeated_limits_never_exhaust_the_attempt_budget() {
    // Arrange: 시도 한도(5)를 훨씬 넘는 횟수만큼 한도에 걸린다
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);

    // Act
    for _ in 0..(MAX_ATTEMPTS * 2) {
        e.ext.script.lock().unwrap().push_back(Err(limit(Some(60))));
        e.env.clock.advance(61_000);
        let out = e.run_next(&|| false).unwrap();
        assert_eq!(out, Ok(JobOutcome::Yielded));
    }

    // Assert
    assert_eq!(attempts(&e), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        0
    );
    assert_eq!(e.st(id, "describe"), "queued");
}

#[test]
fn t_aij_sub_04_while_paused_nothing_is_sent_and_it_resumes_after_the_reset_time() {
    // Arrange: 어댑터가 한도 해제 시각을 알린다
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    let until = T0 + 7_200_000;
    *e.ext.paused_until.lock().unwrap() = Some(until);

    // Act 1: 일시정지 중 - 요청 0건, 시도 0, 해제 시각까지 미룸
    let out = e.run_next(&|| false).unwrap();

    // Assert 1
    assert_eq!(out, Ok(JobOutcome::Yielded));
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert_eq!(attempts(&e), 0);
    assert_eq!(not_before(&e), until);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage"), 0);

    // Act 2: 해제 시각 직전에는 여전히 대기
    e.env.clock.advance(7_199_000);
    assert!(e.run_next(&|| false).is_none());

    // Act 3: 해제 시각이 지나면 (어댑터가 더는 일시정지를 알리지 않는다) 요청이 나가고 완료된다
    e.env.clock.advance(2_000);
    let out = e.run_next(&|| false).unwrap();
    assert!(out.is_ok());
    assert_eq!(e.ext.calls.lock().unwrap().len(), 1);
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aij_sub_05_stale_pause_time_in_the_past_does_not_block() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    *e.ext.paused_until.lock().unwrap() = Some(T0 - 1);

    e.run_next(&|| false).unwrap().unwrap();

    assert_eq!(e.ext.calls.lock().unwrap().len(), 1);
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aij_sub_06_other_retryable_failures_still_count_attempts() {
    // 회귀 대조: 한도가 아닌 재시도 가능 실패는 기존대로 시도를 센다
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("offline", "x")));

    let out = e.run_next(&|| false).unwrap();

    assert!(out.unwrap_err().retryable);
    assert_eq!(attempts(&e), 1);
}

#[test]
fn t_aij_sub_07_failures_before_any_request_leave_no_usage_row() {
    // Arrange: CLI 가 없다 (프로세스를 띄우기 전에 끝난다)
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("cli_not_found", "x")));

    // Act
    e.run_next(&|| false).unwrap().unwrap();

    // Assert: 사용량 0행, 실패 이력은 남는다
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_usage"), 0);
    assert!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0")
            > 0
    );
    assert_eq!(e.st(id, "describe"), "failed");
}

#[test]
fn t_aij_sub_08_failed_request_records_the_actually_billed_mode() {
    // Arrange: 구독 모드로 설정돼 있지만 CLI 가 환경변수 API 키를 썼다
    let e = AiEnv::new();
    *e.ext.auth_mode.lock().unwrap() = Some("subscription".into());
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext.script.lock().unwrap().push_back(Err(
        StepFailure::new("bad_response", "x").with_billed_auth_mode(Some("apiKey".into()))
    ));

    // Act
    e.run_next(&|| false).unwrap().unwrap();

    // Assert
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM ai_usage WHERE auth_mode = 'apiKey' AND ok = 0"),
        1
    );
}

#[test]
fn t_aij_sub_09_failed_request_without_billing_info_uses_the_configured_mode() {
    let e = AiEnv::new();
    *e.ext.auth_mode.lock().unwrap() = Some("subscription".into());
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("bad_response", "x")));

    e.run_next(&|| false).unwrap().unwrap();

    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM ai_usage WHERE auth_mode = 'subscription'"),
        1
    );
}
