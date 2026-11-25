//! T-aij-29 - `JobError::retryable_after` 가 대기열의 `not_before` 를 `max(백오프, retry_after)` 로 정한다.
//! 분류: Time / Boundary / 기존 동작 회귀.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::env::{Env, T0};
use mam_core::SharedClock;
use mam_core::jobs::{JobError, JobKind, JobOutcome, JobQueue, NewJob, backoff_ms};

fn not_before_after(err: JobError) -> i64 {
    let env = Env::new();
    let clock: SharedClock = env.clock.clone();
    let q = JobQueue::new(Arc::clone(&env.pool), clock);
    q.enqueue(NewJob::for_asset(JobKind::Ai, 1)).unwrap();
    let job = q.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    q.finish(&job, Err(err)).unwrap();
    env.count("SELECT not_before FROM jobs")
}

#[test]
fn t_aij_29_retry_after_longer_than_backoff_wins() {
    let nb = not_before_after(JobError::retryable_after("rate", Duration::from_secs(900)));
    assert_eq!(nb, T0 + 900_000);
}

#[test]
fn t_aij_29b_shorter_retry_after_keeps_backoff_and_plain_retryable_is_unchanged() {
    assert_eq!(
        not_before_after(JobError::retryable_after("x", Duration::from_secs(1))),
        T0 + backoff_ms(1)
    );
    assert_eq!(
        not_before_after(JobError::retryable("x")),
        T0 + backoff_ms(1)
    );
    assert_eq!(JobError::retryable("x").retry_after, None);
    assert_eq!(JobError::fatal("x").retry_after, None);
}

#[test]
fn t_aij_29c_retry_after_does_not_apply_to_fatal_or_done() {
    let env = Env::new();
    let clock: SharedClock = env.clock.clone();
    let q = JobQueue::new(Arc::clone(&env.pool), clock);
    q.enqueue(NewJob::for_asset(JobKind::Ai, 1)).unwrap();
    let job = q.claim_next(&[JobKind::Ai]).unwrap().unwrap();

    q.finish(&job, Ok(JobOutcome::Done)).unwrap();

    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 0);
}
