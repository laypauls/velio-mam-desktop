//! P5 종료·중단 (T-pjob-12 / 18) — 앱 종료가 실행 중 작업을 시도 없이 되돌린다.
//!
//! 분류: Concurrency(12 중단 뒤 이어하기: 완료한 provider 는 다시 부르지 않는다) / Time(18 호출 중 종료 -> Yielded, 시도 불변).
//! "재시작" = 같은 DB 위에 새 `JobQueue`(+`recover`)와 `PluginJobs`.

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_core::jobs::JobOutcome;
use mam_plugin_runtime::{AssetKind, InvokeError, ProviderKind};
use serde_json::json;

const P: &str = "com.example.stop";

fn three_providers() -> Plugin {
    Plugin::new(P, "scripted")
        .provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
        .provider("meta.b", ProviderKind::Metadata, &[AssetKind::Image])
        .provider("ana.c", ProviderKind::Analysis, &[AssetKind::Image])
}

#[test]
fn t_pjob_12_stop_after_task_one_of_three_then_rerun_invokes_only_the_remaining_two() {
    // Arrange: 첫 호출이 끝나는 순간 앱이 종료된다
    let mut env = Env::new();
    env.install(three_providers());
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let queue = env.queue.clone();
    env.invoker.set_after(move |c| {
        if c.n == 1 {
            queue.request_shutdown();
        }
    });

    // Act 1: 종료 -> 두 번째 작업 앞에서 양보
    let first = env.run(1);

    // Assert 1
    assert!(matches!(first[0].outcome, Ok(JobOutcome::Yielded)));
    assert_eq!(env.invoker.call_count(), 1);
    assert_eq!(env.pending_tasks(P), 2);
    let rows = env.jobs_rows();
    assert_eq!(
        (rows.len(), rows[0].status.as_str(), rows[0].attempts),
        (1, "pending", 0)
    );

    // Act 2: 앱 재시작 후 이어서 실행
    env.invoker.clear_hooks();
    env.restart();
    env.run(1);

    // Assert 2: provider 3개를 정확히 한 번씩만 호출 (완료한 것을 다시 부르지 않음)
    let mut providers: Vec<String> = env
        .invoker
        .calls()
        .iter()
        .map(|c| c.provider.clone())
        .collect();
    assert_eq!(providers.len(), 3);
    providers.sort();
    assert_eq!(providers, vec!["ana.c", "meta.b", "meta.run"]);
    assert_eq!(env.pending_tasks(P), 0);
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_18_shutdown_during_an_invoke_yields_and_leaves_task_and_job_attempts_unchanged() {
    // Arrange: 실제 워커의 JS 예외가 곧 종료와 겹친다 (종료 신호가 없었다면 script_error 로 실패했을 오류)
    let env = Env::new();
    env.install(Plugin::new(P, "scripted").provider(
        "meta.run",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    env.invoker.script(vec![json!({"throw": "boom"})]);
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let queue = env.queue.clone();
    env.invoker.set_before(move |_| queue.request_shutdown());

    // Act
    let done = env.run(1);

    // Assert: 작업은 실패가 아니라 그대로, 시도 없음
    assert!(matches!(done[0].outcome, Ok(JobOutcome::Yielded)));
    let task = env.task(P, "meta.run", asset).unwrap();
    assert_eq!(
        (task.status.as_str(), task.attempts, task.last_error),
        ("pending", 0, None)
    );
    let rows = env.jobs_rows();
    assert_eq!(
        (rows.len(), rows[0].status.as_str(), rows[0].attempts),
        (1, "pending", 0)
    );
}

#[test]
fn t_pjob_18_transient_errors_during_shutdown_also_yield_without_counting_an_attempt() {
    // Arrange / Act / Assert: 재시도 가능한 오류(시간 초과·크래시·busy)도 종료와 겹치면 시도를 올리지 않는다
    let errors = [
        InvokeError::Timeout,
        InvokeError::Crashed,
        InvokeError::Plugin {
            code: "busy".into(),
            message: "m".into(),
        },
    ];
    for err in errors {
        let env = Env::new();
        env.install(Plugin::new(P, "scripted").provider(
            "meta.run",
            ProviderKind::Metadata,
            &[AssetKind::Image],
        ));
        let asset = env.add_asset("image", "a.jpg");
        env.edit(asset);
        env.jobs.feed_tick();
        let label = err.key();
        env.invoker.inject(vec![err]);
        let queue = env.queue.clone();
        env.invoker.set_before(move |_| queue.request_shutdown());

        let done = env.run(1);

        assert!(
            matches!(done[0].outcome, Ok(JobOutcome::Yielded)),
            "{label}"
        );
        let task = env.task(P, "meta.run", asset).unwrap();
        assert_eq!(
            (task.status.as_str(), task.attempts),
            ("pending", 0),
            "{label}"
        );
        assert_eq!(env.jobs_rows()[0].attempts, 0, "{label}");
    }
}
