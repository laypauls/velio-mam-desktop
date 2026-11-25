//! 작업 대기열 통합 테스트 (T-job-01 ~ 07).
//!
//! 분류: Happy (01) / Business rule (02) / Concurrency (03, 05) / External (04) / Time (06) / End-to-end (07).
//! 실제 SQLite 풀 + 수동 시계. 스레드 시험은 상한(최대 동시 실행 수)만 단언하고 시간 경과로 판정하지 않는다.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::{AtomicClock, Env, wait_until};
use mam_core::SharedClock;
use mam_core::jobs::{
    ExecutorRegistry, Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, JobQueue,
    JobRunner, MAX_ATTEMPTS, NewJob, backoff_ms,
};

const ALL: [JobKind; JobKind::COUNT] = JobKind::ALL;

fn queue(env: &Env) -> Arc<JobQueue> {
    let clock: SharedClock = env.clock.clone();
    JobQueue::new(Arc::clone(&env.pool), clock)
}

fn asset_job(kind: JobKind, asset: i64) -> NewJob {
    NewJob::for_asset(kind, asset)
}

fn status_of(env: &Env, id: i64) -> (String, i64) {
    env.read(|c| {
        c.query_row(
            "SELECT status, attempts FROM jobs WHERE job_id = ?1",
            [id],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    })
}

#[test]
fn t_job_01_runs_in_priority_order_and_respects_per_kind_concurrency() {
    // Arrange: 우선순위가 다른 종류 + 썸네일 3건(상한 2)
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Hash, 1)).unwrap();
    q.enqueue(asset_job(JobKind::Ai, 2)).unwrap();
    q.enqueue(asset_job(JobKind::Thumb, 3)).unwrap();
    q.enqueue(asset_job(JobKind::Thumb, 4)).unwrap();
    q.enqueue(asset_job(JobKind::Thumb, 5)).unwrap();

    // Act
    let order: Vec<(JobKind, Option<i64>)> = std::iter::from_fn(|| q.claim_next(&ALL).unwrap())
        .map(|j| (j.kind, j.asset_id))
        .collect();

    // Assert: 썸네일(20) 2건 -> 상한 도달로 3번째는 건너뛰고 해시(30) -> AI(50)
    assert_eq!(
        order,
        vec![
            (JobKind::Thumb, Some(3)),
            (JobKind::Thumb, Some(4)),
            (JobKind::Hash, Some(1)),
            (JobKind::Ai, Some(2)),
        ]
    );
    assert_eq!(q.running_count(JobKind::Thumb), 2);
    assert!(q.claim_next(&ALL).unwrap().is_none(), "thumb cap reached");
}

#[test]
fn t_job_01b_a_finished_slot_lets_the_next_job_start_and_same_priority_is_fifo() {
    let env = Env::new();
    let q = queue(&env);
    for a in 1..=3 {
        q.enqueue(asset_job(JobKind::Hash, a)).unwrap();
    }
    let first = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(first.asset_id, Some(1));
    assert!(q.claim_next(&ALL).unwrap().is_none(), "hash cap is 1");

    q.finish(&first, Ok(JobOutcome::Done)).unwrap();
    let second = q.claim_next(&ALL).unwrap().unwrap();

    assert_eq!(second.asset_id, Some(2));
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'running'"),
        1
    );
}

#[test]
fn t_job_01c_kinds_without_an_executor_are_never_claimed() {
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Thumb, 1)).unwrap();

    assert!(q.claim_next(&[JobKind::Scan]).unwrap().is_none());
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'pending'"),
        1
    );
}

#[test]
fn t_job_02_paused_kinds_do_not_start_and_resume_continues() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Hash, 1)).unwrap();
    q.enqueue(asset_job(JobKind::Thumb, 2)).unwrap();

    // Act / Assert: 종류별 일시정지
    q.pause(Some(JobKind::Thumb)).unwrap();
    assert!(q.is_paused(JobKind::Thumb) && !q.is_paused(JobKind::Hash));
    assert_eq!(q.paused_kinds(), vec![JobKind::Thumb]);
    let j = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(j.kind, JobKind::Hash, "other kinds keep running");
    q.finish(&j, Ok(JobOutcome::Done)).unwrap();
    assert!(q.claim_next(&ALL).unwrap().is_none());

    // 전역 일시정지는 종류별 재개로 풀리지 않는다
    q.resume(Some(JobKind::Thumb)).unwrap();
    q.pause(None).unwrap();
    assert!(q.paused_all() && q.claim_next(&ALL).unwrap().is_none());

    // 재개
    q.resume(None).unwrap();
    assert!(!q.paused_all() && q.paused_kinds().is_empty());
    assert_eq!(q.claim_next(&ALL).unwrap().unwrap().kind, JobKind::Thumb);
}

#[test]
fn t_job_02b_yielded_job_returns_to_pending_without_using_an_attempt() {
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Hash, 1)).unwrap();
    let j = q.claim_next(&ALL).unwrap().unwrap();

    q.finish(&j, Ok(JobOutcome::Yielded)).unwrap();

    assert_eq!(status_of(&env, j.job_id), ("pending".into(), 0));
    assert_eq!(q.running_count(JobKind::Hash), 0);
    assert_eq!(q.claim_next(&ALL).unwrap().unwrap().job_id, j.job_id);
}

#[test]
fn t_job_03_running_jobs_return_to_pending_after_restart_without_double_execution() {
    // Arrange: 실행 중이던 작업을 남기고 "재시작" (새 큐 인스턴스 = 메모리 상태 없음)
    let env = Env::new();
    let before = queue(&env);
    before.enqueue(asset_job(JobKind::Hash, 7)).unwrap();
    let running = before.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(status_of(&env, running.job_id).0, "running");

    // Act
    let after = queue(&env);
    let recovered = after.recover().unwrap();
    let again = after.claim_next(&ALL).unwrap().unwrap();

    // Assert: 같은 작업이 다시 대기 -> 한 번만 claim 된다
    assert_eq!(recovered, 1);
    assert_eq!(again.job_id, running.job_id);
    assert!(
        after.claim_next(&ALL).unwrap().is_none(),
        "no duplicate execution"
    );
}

#[test]
fn t_job_03b_restart_survives_a_pending_twin_of_a_running_job() {
    // Arrange: 실행 중인 (meta, 자산 7) 작업이 있는 동안 같은 작업이 다시 등록됐다 (대기 UNIQUE 는 running 을 막지 않는다)
    let env = Env::new();
    let before = queue(&env);
    before.enqueue(asset_job(JobKind::Meta, 7)).unwrap();
    let running = before.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(status_of(&env, running.job_id).0, "running");
    assert!(before.enqueue(asset_job(JobKind::Meta, 7)).unwrap());
    before.enqueue(asset_job(JobKind::Meta, 8)).unwrap();

    // Act: 비정상 종료 뒤 재시작 — 예전에는 UNIQUE 위반으로 시작이 영구히 실패했다
    let after = queue(&env);
    let recovered = after
        .recover()
        .expect("recover must not violate the pending unique index");

    // Assert: 대기 쌍둥이가 있는 running 행(7)은 버려지고, 나머지는 대기로 돌아온다
    assert_eq!(recovered, 0, "the only running row had a pending twin");
    let rows: Vec<(i64, String)> = env.read(|c| {
        let mut st = c
            .prepare("SELECT asset_id, status FROM jobs ORDER BY asset_id, job_id")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    assert_eq!(
        rows.iter().filter(|(a, _)| *a == 7).count(),
        1,
        "one job for asset 7"
    );
    assert!(rows.iter().all(|(_, s)| s == "pending"), "{rows:?}");
}

#[test]
fn t_job_04_failures_back_off_then_fail_after_max_attempts_and_can_be_retried() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Hash, 1)).unwrap();

    // Act / Assert: 실패할 때마다 백오프만큼 시작이 미뤄진다
    for attempt in 1..MAX_ATTEMPTS {
        let j = q.claim_next(&ALL).unwrap().unwrap();
        q.finish(&j, Err(JobError::retryable("boom"))).unwrap();
        assert_eq!(status_of(&env, j.job_id), ("pending".into(), attempt));
        assert!(q.claim_next(&ALL).unwrap().is_none(), "backing off");
        env.clock.advance(backoff_ms(attempt) - 1);
        assert!(q.claim_next(&ALL).unwrap().is_none(), "1 ms early");
        env.clock.advance(1);
    }
    let last = q.claim_next(&ALL).unwrap().unwrap();
    q.finish(&last, Err(JobError::retryable("boom again")))
        .unwrap();

    // Assert: 5회째 실패는 failed (자동 재시도 중단)
    assert_eq!(
        status_of(&env, last.job_id),
        ("failed".into(), MAX_ATTEMPTS)
    );
    env.clock.advance(10 * 3_600_000);
    assert!(q.claim_next(&ALL).unwrap().is_none());
    let counts = q.counts().unwrap();
    assert_eq!(counts[3].0, JobKind::Hash);
    assert_eq!(counts[3].1.failed, 1);

    // 수동 재시도
    assert_eq!(q.retry_failed(None).unwrap(), 1);
    assert_eq!(status_of(&env, last.job_id), ("pending".into(), 0));
    assert!(q.claim_next(&ALL).unwrap().is_some());
}

#[test]
fn t_job_04b_non_retryable_error_fails_immediately_and_backoff_is_capped() {
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(asset_job(JobKind::Ai, 1)).unwrap();
    let j = q.claim_next(&ALL).unwrap().unwrap();

    q.finish(&j, Err(JobError::fatal("bad input"))).unwrap();

    assert_eq!(status_of(&env, j.job_id), ("failed".into(), 1));
    assert_eq!(backoff_ms(1), 30_000);
    assert_eq!(backoff_ms(2), 60_000);
    assert_eq!(backoff_ms(60), 3_600_000, "capped at one hour, no overflow");
}

#[test]
fn t_job_05_duplicate_pending_jobs_are_not_registered() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);

    // Act
    let first = q.enqueue(asset_job(JobKind::Thumb, 1)).unwrap();
    let dup = q.enqueue(asset_job(JobKind::Thumb, 1)).unwrap();
    let other_kind = q.enqueue(asset_job(JobKind::Meta, 1)).unwrap();
    let other_asset = q.enqueue(asset_job(JobKind::Thumb, 2)).unwrap();

    // Assert
    assert!([first, !dup, other_kind, other_asset].iter().all(|b| *b));
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 3);

    // 실행 중에는 다음 변경분을 위한 대기 작업을 하나 더 둘 수 있다
    let running = q.claim_next(&[JobKind::Thumb]).unwrap().unwrap();
    assert_eq!(running.asset_id, Some(1));
    assert!(q.enqueue(asset_job(JobKind::Thumb, 1)).unwrap());
    assert!(!q.enqueue(asset_job(JobKind::Thumb, 1)).unwrap());
}

#[test]
fn t_job_06_not_before_delays_the_start() {
    let env = Env::new();
    let q = queue(&env);
    let at = env.clock_now() + 60_000;
    q.enqueue(NewJob {
        not_before: at,
        ..asset_job(JobKind::Hash, 1)
    })
    .unwrap();
    q.enqueue(asset_job(JobKind::Hash, 2)).unwrap();

    // 예약 시각 전에는 나중에 등록된 즉시 작업이 먼저 나간다
    let now_job = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(now_job.asset_id, Some(2));
    q.finish(&now_job, Ok(JobOutcome::Done)).unwrap();
    assert!(q.claim_next(&ALL).unwrap().is_none());

    env.clock.advance(59_999);
    assert!(q.claim_next(&ALL).unwrap().is_none());
    env.clock.advance(1);
    assert_eq!(q.claim_next(&ALL).unwrap().unwrap().asset_id, Some(1));
}

/// 동시에 실행 중인 수의 최대값을 기록하는 실행기
struct Probe {
    current: AtomicUsize,
    max: AtomicUsize,
    done: Mutex<Vec<i64>>,
}

impl JobExecutor for Probe {
    fn execute(&self, job: &Job, _ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        self.current.fetch_sub(1, Ordering::SeqCst);
        self.done
            .lock()
            .unwrap()
            .push(job.asset_id.unwrap_or_default());
        Ok(JobOutcome::Done)
    }
}

#[test]
fn t_job_07_runner_executes_all_jobs_within_the_concurrency_cap_and_pause_blocks_start() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    let probe = Arc::new(Probe {
        current: AtomicUsize::new(0),
        max: AtomicUsize::new(0),
        done: Mutex::new(Vec::new()),
    });
    let mut reg = ExecutorRegistry::new();
    reg.register(JobKind::Thumb, probe.clone());
    q.pause(Some(JobKind::Thumb)).unwrap();
    for a in 1..=8 {
        q.enqueue(asset_job(JobKind::Thumb, a)).unwrap();
    }
    let runner = JobRunner::start(Arc::clone(&q), reg).unwrap();

    // Act 1: 일시정지 중에는 시작하지 않는다 (디스패처가 여러 번 돌 시간을 준다)
    std::thread::sleep(Duration::from_millis(300));
    assert!(probe.done.lock().unwrap().is_empty());
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'pending'"),
        8
    );

    // Act 2: 재개
    q.resume(Some(JobKind::Thumb)).unwrap();
    let finished = wait_until(Duration::from_secs(20), || {
        probe.done.lock().unwrap().len() == 8
    });
    runner.stop();

    // Assert
    assert!(finished, "all jobs should run");
    assert!(probe.max.load(Ordering::SeqCst) <= JobKind::Thumb.default_concurrency());
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs"),
        0,
        "done jobs are removed"
    );
    let mut done = probe.done.lock().unwrap().clone();
    done.sort_unstable();
    assert_eq!(done, (1..=8).collect::<Vec<_>>());
}

#[test]
fn t_job_07b_executor_panic_is_a_retryable_failure_not_a_crash() {
    struct Boom;
    impl JobExecutor for Boom {
        fn execute(&self, _: &Job, _: &JobContext<'_>) -> Result<JobOutcome, JobError> {
            panic!("executor bug");
        }
    }
    let env = Env::new();
    let q = queue(&env);
    let mut reg = ExecutorRegistry::new();
    reg.register(JobKind::Hash, Arc::new(Boom));
    q.enqueue(asset_job(JobKind::Hash, 1)).unwrap();
    let runner = JobRunner::start(Arc::clone(&q), reg).unwrap();

    let failed_once = wait_until(Duration::from_secs(10), || {
        env.count("SELECT COUNT(*) FROM jobs WHERE attempts = 1 AND status = 'pending'") == 1
    });
    runner.stop();

    assert!(failed_once);
    let msg: String = env.read(|c| {
        c.query_row("SELECT last_error FROM jobs", [], |r| r.get(0))
            .unwrap()
    });
    assert!(msg.contains("panicked"));
}

#[allow(dead_code)]
fn _clock_type_check(_: &AtomicClock) {}
