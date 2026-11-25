//! 작업별 취소 (`JobQueue::cancel_running` / `wait_jobs_finished`) — USB 꺼내기 준비의 기반 (T-jobcancel-01 ~ 04).
//!
//! 분류: Happy (01 취소→양보→대기 복귀) / Boundary (02 모르는 ID·이미 끝난 작업) / Concurrency (03 대기 해제·시간 초과) /
//! Business rule (04 종료 요청도 같은 플래그). 실제 SQLite + 수동 시계.
#![cfg(windows)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use common::env::Env;
use mam_core::SharedClock;
use mam_core::jobs::{
    ExecutorRegistry, Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, JobQueue,
    JobRunner, NewJob,
};

const WAIT: Duration = Duration::from_secs(20);

/// 취소(`should_stop`) 또는 `shutdown_flag` 가 켜질 때까지 돌다 `Yielded`.
struct Spin {
    entered: Sender<i64>,
    saw_flag: Arc<AtomicBool>,
}

impl JobExecutor for Spin {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let _ = self.entered.send(job.job_id);
        while !ctx.should_stop() {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.saw_flag
            .store(ctx.shutdown_flag().load(Ordering::SeqCst), Ordering::SeqCst);
        // 한 번 양보한 뒤에는 다시 잡혀도 바로 끝낸다 (이 시험은 첫 실행만 본다)
        Ok(JobOutcome::Yielded)
    }
}

fn runner(
    env: &Env,
) -> (
    Arc<JobQueue>,
    JobRunner,
    std::sync::mpsc::Receiver<i64>,
    Arc<AtomicBool>,
) {
    let clock: SharedClock = env.clock.clone();
    let q = JobQueue::new(Arc::clone(&env.pool), clock);
    let (tx, rx) = channel();
    let saw = Arc::new(AtomicBool::new(false));
    let mut reg = ExecutorRegistry::new();
    reg.register(
        JobKind::Thumb,
        Arc::new(Spin {
            entered: tx,
            saw_flag: Arc::clone(&saw),
        }),
    );
    let r = JobRunner::start(Arc::clone(&q), reg).unwrap();
    (q, r, rx, saw)
}

#[test]
fn t_jobcancel_01_cancelling_a_running_job_makes_it_yield_back_to_pending_without_an_attempt() {
    // Arrange
    let env = Env::new();
    let (q, r, rx, saw) = runner(&env);
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 1)).unwrap();
    let id = rx.recv_timeout(WAIT).unwrap();
    assert_eq!(q.running_jobs().len(), 1);

    // Act
    assert_eq!(q.cancel_running(&[id]), 1);
    let done = q.wait_jobs_finished(&[id], WAIT);

    // Assert: 결과가 DB 에 반영된 뒤에 대기가 풀리고, 시도 횟수는 그대로이며 취소 신호는 `shutdown_flag` 로도 보인다
    assert!(done);
    assert!(
        saw.load(Ordering::SeqCst),
        "cancel is visible through shutdown_flag()"
    );
    let (attempts, failed): (i64, i64) = env.read(|c| {
        (
            c.query_row("SELECT attempts FROM jobs WHERE job_id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap(),
            c.query_row(
                "SELECT COUNT(*) FROM jobs WHERE status = 'failed'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
        )
    });
    assert_eq!((attempts, failed), (0, 0));
    assert!(!q.is_shutdown(), "a per-job cancel is not a shutdown");
    r.stop();
}

#[test]
fn t_jobcancel_02_unknown_and_finished_job_ids_are_ignored() {
    // Arrange
    let env = Env::new();
    let (q, r, _rx, _saw) = runner(&env);

    // Act / Assert
    assert_eq!(q.cancel_running(&[12345]), 0);
    assert!(
        q.wait_jobs_finished(&[12345], Duration::from_millis(10)),
        "nothing to wait for"
    );
    assert!(q.running_jobs().is_empty());
    r.stop();
}

#[test]
fn t_jobcancel_03_wait_times_out_while_the_job_ignores_the_cancel_and_other_jobs_are_untouched() {
    // Arrange: 취소를 무시하는 실행기 (응답 없는 작업) + 건드리면 안 되는 두 번째 작업
    struct Stubborn(Sender<i64>, Arc<AtomicBool>);
    impl JobExecutor for Stubborn {
        fn execute(&self, job: &Job, _ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
            let _ = self.0.send(job.job_id);
            while !self.1.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            Ok(JobOutcome::Done)
        }
    }
    let env = Env::new();
    let clock: SharedClock = env.clock.clone();
    let q = JobQueue::new(Arc::clone(&env.pool), clock);
    let (tx, rx) = channel();
    let release = Arc::new(AtomicBool::new(false));
    let mut reg = ExecutorRegistry::new();
    reg.register(JobKind::Thumb, Arc::new(Stubborn(tx, Arc::clone(&release))));
    let r = JobRunner::start(Arc::clone(&q), reg).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 1)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 2)).unwrap();
    let (a, b) = (
        rx.recv_timeout(WAIT).unwrap(),
        rx.recv_timeout(WAIT).unwrap(),
    );

    // Act
    q.cancel_running(&[a]);
    let timed_out = !q.wait_jobs_finished(&[a], Duration::from_millis(60));
    release.store(true, Ordering::SeqCst);
    let finished = q.wait_jobs_finished(&[a, b], WAIT);

    // Assert
    assert!(timed_out, "bounded wait gives up while the job still runs");
    assert!(finished, "waiters are released once the jobs end");
    r.stop();
}

#[test]
fn t_jobcancel_04_shutdown_request_raises_the_same_flag_for_jobs_already_running() {
    // Arrange
    let env = Env::new();
    let (q, r, rx, saw) = runner(&env);
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 1)).unwrap();
    let id = rx.recv_timeout(WAIT).unwrap();

    // Act
    q.request_shutdown();
    let done = q.wait_jobs_finished(&[id], WAIT);

    // Assert
    assert!(done);
    assert!(saw.load(Ordering::SeqCst));
    r.stop();
}
