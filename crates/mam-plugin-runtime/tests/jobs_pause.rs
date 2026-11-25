//! P5 일시정지 (T-pjob-11) — 피드·실행기가 `Plugin` 종류 일시정지를 듣는다.
//!
//! 분류: Time(일시정지·재개) / Business rule(작업 사이에서 Yielded, 시도 없음) / Concurrency+Regression(실행 중 쌍둥이 작업 + Yielded).
//! 실제 SQLite·실제 워커·실제 `JobQueue`. 일시정지 시점은 provider 호출 훅으로 정확히 정한다.

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_core::jobs::{JobKind, JobOutcome};
use mam_plugin_runtime::{AssetKind, ProviderKind};

const P: &str = "com.example.pause";

/// 같은 자산에 작업 2개(provider 2개)가 생기는 플러그인
fn two_providers() -> Plugin {
    Plugin::new(P, "scripted")
        .provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
        .provider("meta.b", ProviderKind::Metadata, &[AssetKind::Image])
}

fn pause(env: &Env) {
    env.queue.pause(Some(JobKind::Plugin)).unwrap();
}

#[test]
fn t_pjob_11_paused_feed_does_not_advance_the_cursor_and_resume_feeds_the_backlog() {
    // Arrange
    let env = Env::new();
    env.install(two_providers());
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    pause(&env);

    // Act 1: 일시정지 중 틱
    let more = env.jobs.feed_tick();

    // Assert 1: 커서도 작업도 움직이지 않는다
    assert!(!more);
    assert_eq!((env.cursor(P), env.task_rows(P), env.jobs_rows().len()), (Some(0), 0, 0));

    // Act 2: 재개
    env.queue.resume(Some(JobKind::Plugin)).unwrap();
    env.jobs.feed_tick();

    // Assert 2
    assert_eq!((env.cursor(P), env.task_rows(P), env.jobs_rows().len()), (Some(env.max_event()), 2, 1));
}

#[test]
fn t_pjob_11_pause_between_tasks_yields_without_an_attempt_and_resume_runs_only_the_rest() {
    // Arrange: 첫 provider 호출이 끝나는 순간 일시정지
    let env = Env::new();
    env.install(two_providers());
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let queue = env.queue.clone();
    env.invoker.set_after(move |c| {
        if c.n == 1 {
            queue.pause(Some(JobKind::Plugin)).unwrap();
        }
    });

    // Act 1
    let first = env.run(1);

    // Assert 1: 작업 사이에서 Yielded — 두 번째 provider 는 호출되지 않았고 시도도 올리지 않았다
    assert!(matches!(first[0].outcome, Ok(JobOutcome::Yielded)));
    assert_eq!(env.invoker.call_count(), 1);
    let rows = env.jobs_rows();
    assert_eq!((rows.len(), rows[0].status.as_str(), rows[0].attempts), (1, "pending", 0));
    let done_first = env.invoker.calls()[0].provider.clone();
    let other = if done_first == "meta.run" { "meta.b" } else { "meta.run" };
    assert_eq!(env.task(P, &done_first, asset).unwrap().status, "done");
    let rest = env.task(P, other, asset).unwrap();
    assert_eq!((rest.status.as_str(), rest.attempts, rest.last_error), ("pending", 0, None));

    // Act 2: 재개
    env.invoker.clear_hooks();
    env.queue.resume(Some(JobKind::Plugin)).unwrap();
    env.run(1);

    // Assert 2: 남은 하나만 호출 — 완료된 provider 를 다시 부르지 않는다
    let providers: Vec<String> = env.invoker.calls().iter().map(|c| c.provider.clone()).collect();
    assert_eq!(providers, vec![done_first, other.to_owned()]);
    assert_eq!(env.pending_tasks(P), 0);
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_11_yield_with_a_twin_job_registered_during_the_run_does_not_leave_a_stuck_running_row() {
    // Arrange: 실행 중(첫 호출 직후)에 사용자가 편집하고 피드가 같은 자산의 작업을 다시 올린 뒤 일시정지가 걸린다.
    // 부분 UNIQUE(uq_jobs_pending_asset)는 running 행을 막지 않으므로 쌍둥이 대기 작업이 생기고, 예전에는 Yielded 의
    // pending 복귀가 UNIQUE 위반으로 실패해 작업이 영구히 running 으로 굳었다.
    let env = Env::new();
    env.install(two_providers());
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let (editor, jobs, queue) = (env.editor(), env.jobs.clone(), env.queue.clone());
    env.invoker.set_after(move |c| {
        if c.n == 1 {
            editor.edit(c.asset);
            jobs.feed_tick(); // 이 시점엔 아직 일시정지 전 — 쌍둥이 작업이 올라간다
            queue.pause(Some(JobKind::Plugin)).unwrap();
        }
    });

    // Act
    let first = env.run(1);

    // Assert: Yielded 가 오류 없이 끝나 running 행이 남지 않고, 쌍둥이 하나가 이어받는다
    assert!(matches!(first[0].outcome, Ok(JobOutcome::Yielded)));
    let rows = env.jobs_rows();
    assert_eq!(rows.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(), vec!["pending"], "{rows:?}");
    assert_eq!(env.queue.running_count(JobKind::Plugin), 0);

    // Act 2: 재개 -> 새 revision(3)까지 모두 처리된다
    env.invoker.clear_hooks();
    env.queue.resume(Some(JobKind::Plugin)).unwrap();
    env.run(1);

    // Assert 2
    assert_eq!(env.pending_tasks(P), 0);
    assert!(["meta.run", "meta.b"].iter().all(|p| env.task(P, p, asset).is_some_and(|t| t.status == "done" && t.done == 3)));
    assert!(env.jobs_rows().is_empty());
}
