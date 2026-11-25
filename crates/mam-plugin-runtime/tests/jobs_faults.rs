//! P5 외부 실패 (T-pjob-13 / 14 일부) — 워커가 PID 로 종료됨·시간 초과·JS 예외·재시도 한도.
//!
//! 분류: External(실제 워커 프로세스 실패) / Time(백오프는 주입 시계로 건너뜀) / Business rule(재시도 한도 -> failed).
//! 워커 종료는 시험이 띄운 워커의 **PID 로만** 한다 (이미지 이름으로 종료하지 않는다). 차단기는 크게 열어 자동 비활성을 막는다.

mod common_jobs;

use std::time::Duration;

use common_jobs::{Env, Plugin, pid_alive};
use mam_core::jobs::{JobOutcome, MAX_ATTEMPTS};
use mam_plugin_host::BreakerConfig;
use mam_plugin_runtime::{AssetKind, ProviderKind};
use serde_json::json;

const P: &str = "com.example.faults";

fn scripted() -> Plugin {
    let mut p = Plugin::new(P, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image]);
    p.breaker = Some(BreakerConfig { max_faults: 100, window: Duration::from_secs(600) });
    p
}

fn queued_asset(env: &Env) -> i64 {
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    asset
}

#[test]
fn t_pjob_13_worker_killed_by_pid_mid_task_is_retried_with_backoff_and_then_succeeds() {
    // Arrange: 첫 시도는 워커가 자기 PID 로 종료당하고, 재시도는 정상 출력
    let env = Env::new();
    env.install(scripted());
    env.invoker.script(vec![json!({"kill": true}), json!({"out": {"kv": {"ok": 1}}})]);
    let asset = queued_asset(&env);
    let pid = env.invoker.arm_kill(P);

    // Act 1: 호출 도중 워커가 죽는다
    let first = env.run(1);

    // Assert 1: 재시도 가능 오류 — 작업은 대기에 남고 시도 1, 작업 행은 백오프로 미뤄짐
    assert!(first[0].outcome.as_ref().is_err_and(|e| e.retryable));
    assert!(!pid_alive(pid), "the worker was killed by its PID");
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((task.status.as_str(), task.attempts, env.kv_rows()), ("pending", 1, 0));
    let rows = env.jobs_rows();
    assert_eq!((rows[0].status.as_str(), rows[0].attempts), ("pending", 1));
    assert!(rows[0].not_before > common_jobs::T0, "backoff applied");

    // Act 2: 백오프를 건너뛰어 재시도 (새 워커)
    env.skip_backoff();
    let second = env.run(1);

    // Assert 2
    assert!(matches!(second[0].outcome, Ok(JobOutcome::Done)));
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((task.status.as_str(), task.attempts), ("done", 0));
    assert_eq!(env.kv(P, asset, "ok").as_deref(), Some("1"));
    assert!(env.jobs_rows().is_empty());
    assert_eq!((env.invoker.call_count(), env.invoker.host(P).starts()), (2, 2));
}

#[test]
fn t_pjob_13_after_max_attempts_of_worker_crashes_the_task_is_failed_and_the_job_is_done() {
    // Arrange: 매 시도마다 워커가 죽는다
    let env = Env::new();
    env.install(scripted());
    let asset = queued_asset(&env);
    let mut pids = Vec::new();

    // Act: MAX_ATTEMPTS 번 시도 (매번 새 워커의 PID 를 알아 두고 종료)
    for attempt in 1..=MAX_ATTEMPTS {
        env.invoker.script(vec![json!({"kill": true})]);
        pids.push(env.invoker.arm_kill(P));
        let done = env.run(1);
        if attempt < MAX_ATTEMPTS {
            assert!(done[0].outcome.as_ref().is_err_and(|e| e.retryable), "attempt {attempt} is retried");
            env.skip_backoff();
        } else {
            assert!(matches!(done[0].outcome, Ok(JobOutcome::Done)), "the last attempt closes the job");
        }
    }

    // Assert: 작업은 failed(crashed), 대기 작업 없음, 호출은 정확히 MAX_ATTEMPTS 번
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((task.status.as_str(), task.last_error.as_deref(), task.attempts), ("failed", Some("crashed"), MAX_ATTEMPTS));
    assert_eq!(env.invoker.call_count(), MAX_ATTEMPTS as usize);
    assert!(env.jobs_rows().is_empty());
    assert!(pids.iter().all(|p| !pid_alive(*p)));
}

#[test]
fn t_pjob_14_timeout_is_retried_and_the_worker_is_discarded() {
    // Arrange: 호출 시간 상한 0.4 s, CPU 한도는 그보다 길게 -> 무한 루프가 시간 초과로 끝난다
    let env = Env::new();
    let mut p = scripted();
    p.call_timeout = Some(Duration::from_millis(400));
    p.cpu_ms = Some(20_000);
    env.install(p);
    env.invoker.script(vec![json!({"spin": true})]);
    let asset = queued_asset(&env);

    // Act
    let done = env.run(1);

    // Assert: 재시도 가능 — 대기 유지, 시도 1
    assert!(done[0].outcome.as_ref().is_err_and(|e| e.retryable));
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((task.status.as_str(), task.attempts, env.kv_rows()), ("pending", 1, 0));
    assert_eq!(env.jobs_rows()[0].attempts, 1);
}

#[test]
fn t_pjob_14_js_exception_fails_the_task_permanently_without_retry() {
    // Arrange
    let env = Env::new();
    env.install(scripted());
    env.invoker.script(vec![json!({"throw": "boom"})]);
    let asset = queued_asset(&env);

    // Act
    let done = env.run(1);

    // Assert: failed(script_error), 작업은 정상 종료(Done) — 재시도 없음
    assert!(matches!(done[0].outcome, Ok(JobOutcome::Done)));
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!((task.status.as_str(), task.last_error.as_deref(), task.attempts), ("failed", Some("script_error"), 0));
    assert!(env.jobs_rows().is_empty());
    assert_eq!(env.invoker.call_count(), 1);
}
