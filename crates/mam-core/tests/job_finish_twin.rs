//! 작업 대기열 `finish` 회귀 — 실행 중인 작업과 같은 (kind, 자산) 의 대기 작업이 그 사이 다시 등록된 경우 (T-job-08).
//!
//! 분류: Concurrency(실행 중 재등록) / Business rule(Yielded·재시도 가능 오류) / Regression.
//! 부분 UNIQUE `uq_jobs_pending_asset` 은 running 행을 막지 않으므로 변경 피드가 실행 중 자산의 작업을 다시 올릴 수 있다.
//! 예전에는 `finish` 가 행을 pending 으로 되돌리다 UNIQUE 위반으로 실패해 행이 영구히 `running` 으로 남았다.

mod common;

use std::sync::Arc;

use common::env::Env;
use mam_core::SharedClock;
use mam_core::jobs::{JobError, JobKind, JobOutcome, JobQueue, NewJob};

fn queue(env: &Env) -> Arc<JobQueue> {
    let clock: SharedClock = env.clock.clone();
    JobQueue::new(Arc::clone(&env.pool), clock)
}

fn rows(env: &Env) -> Vec<(i64, String)> {
    env.read(|c| {
        let mut st = c
            .prepare("SELECT asset_id, status FROM jobs ORDER BY asset_id, job_id")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

/// 실행 중인 (plugin, 자산 7) 작업과 그 대기 쌍둥이를 만든다
fn running_with_twin(q: &JobQueue) -> mam_core::jobs::Job {
    q.enqueue(NewJob::for_asset(JobKind::Plugin, 7)).unwrap();
    let running = q.claim_next(&JobKind::ALL).unwrap().unwrap();
    assert!(
        q.enqueue(NewJob::for_asset(JobKind::Plugin, 7)).unwrap(),
        "pending twin is allowed while the first is running"
    );
    running
}

#[test]
fn t_job_08_yielded_job_with_a_pending_twin_finishes_cleanly_and_leaves_one_pending_row() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    let running = running_with_twin(&q);

    // Act
    let result = q.finish(&running, Ok(JobOutcome::Yielded));

    // Assert: 예전에는 UNIQUE 위반 오류 + 행이 running 으로 굳음
    result.expect("finish must not fail on a pending twin");
    assert_eq!(
        rows(&env),
        vec![(7, "pending".to_owned())],
        "the twin takes over, no stuck running row"
    );
    assert_eq!(q.running_count(JobKind::Plugin), 0);
}

#[test]
fn t_job_08_retryable_error_with_a_pending_twin_finishes_cleanly_and_leaves_one_pending_row() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    let running = running_with_twin(&q);

    // Act
    let result = q.finish(&running, Err(JobError::retryable("transient")));

    // Assert
    result.expect("finish must not fail on a pending twin");
    assert_eq!(rows(&env), vec![(7, "pending".to_owned())]);
    let again = q
        .claim_next(&JobKind::ALL)
        .unwrap()
        .expect("the twin is claimable");
    assert_eq!(again.asset_id, Some(7));
}

#[test]
fn t_job_08_without_a_twin_yielded_and_retry_still_return_the_row_to_pending() {
    // Arrange / Act / Assert: 쌍둥이가 없으면 예전과 같다 (행 유지, 시도 횟수 규칙 유지)
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Plugin, 7)).unwrap();
    let j = q.claim_next(&JobKind::ALL).unwrap().unwrap();
    q.finish(&j, Ok(JobOutcome::Yielded)).unwrap();
    assert_eq!(rows(&env), vec![(7, "pending".to_owned())]);

    let j = q.claim_next(&JobKind::ALL).unwrap().unwrap();
    q.finish(&j, Err(JobError::retryable("transient"))).unwrap();
    let attempts: i64 = env.read(|c| {
        c.query_row(
            "SELECT attempts FROM jobs WHERE job_id = ?1",
            [j.job_id],
            |r| r.get(0),
        )
        .unwrap()
    });
    assert_eq!((rows(&env), attempts), (vec![(7, "pending".to_owned())], 1));
}

#[test]
fn t_job_08_a_twin_of_another_kind_or_asset_does_not_swallow_the_job() {
    // Arrange: 다른 종류(meta)·다른 자산(8)의 대기 작업은 쌍둥이가 아니다
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Plugin, 7)).unwrap();
    let running = q.claim_next(&JobKind::ALL).unwrap().unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Meta, 7)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Plugin, 8)).unwrap();

    // Act
    q.finish(&running, Ok(JobOutcome::Yielded)).unwrap();

    // Assert: 7(plugin) 행이 살아 있다 -> 자산 7 에 2행(meta + plugin), 자산 8 에 1행
    assert_eq!(rows(&env).iter().filter(|(a, _)| *a == 7).count(), 2);
    assert_eq!(rows(&env).iter().filter(|(a, _)| *a == 8).count(), 1);
}
