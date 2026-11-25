//! 작업 러너 구동 — `Probe` 로 감싼 플러그인 실행기 + 끝난 작업 채널 + 큐 condvar 대기 (sleep 폴링 없음).

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::jobs::{ExecutorRegistry, Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, JobRunner};

use super::Env;

/// 실행기가 끝낸 작업 1건 (`JobRunner` 가 `finish` 를 마치기 직전의 결과)
#[derive(Debug, Clone)]
pub struct Done {
    pub job_id: i64,
    pub asset: Option<i64>,
    pub attempts: i64,
    pub outcome: Result<JobOutcome, JobError>,
}

struct Probe {
    inner: Arc<dyn JobExecutor>,
    tx: Mutex<Sender<Done>>,
}

impl JobExecutor for Probe {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let outcome = self.inner.execute(job, ctx);
        let _ = self.tx.lock().unwrap().send(Done { job_id: job.job_id, asset: job.asset_id, attempts: job.attempts, outcome: outcome.clone() });
        outcome
    }
}

impl Env {
    // ── 작업 대기열 ──────────────────────────────────────────

    /// 러너가 없으면 `Probe` 로 감싼 플러그인 실행기를 등록해 시작한다
    pub fn start_runner(&self) {
        let mut slot = self.runner.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let mut reg = ExecutorRegistry::new();
        let probe = Probe { inner: self.jobs.executor(), tx: Mutex::new(self.done_tx.clone()) };
        reg.register(JobKind::Plugin, Arc::new(probe));
        *slot = Some(JobRunner::start(Arc::clone(&self.queue), reg).unwrap());
    }

    pub fn stop_runner(&self) {
        if let Some(r) = self.runner.lock().unwrap().take() {
            r.stop();
        }
    }

    /// 작업 `n` 건이 끝날 때까지 기다린다 (러너 시작 포함). 끝난 작업 목록을 돌려준다 — `finish` 까지 마친 뒤
    pub fn run(&self, n: usize) -> Vec<Done> {
        self.start_runner();
        let done: Vec<Done> = (0..n).map(|_| self.next_done()).collect();
        self.settle();
        done
    }

    pub fn next_done(&self) -> Done {
        self.done_rx.lock().unwrap().recv_timeout(Duration::from_secs(60)).expect("a plugin job should finish within 60s")
    }

    /// 실행 중 플러그인 작업이 없어질 때까지 (= `finish` 의 DB 쓰기 완료) 큐의 condvar 로 기다린다
    pub fn settle(&self) {
        let (limit, mut seen) = (Instant::now() + Duration::from_secs(30), 0u64);
        while self.queue.running_count(JobKind::Plugin) > 0 {
            assert!(Instant::now() < limit, "plugin job did not settle");
            self.queue.wait(&mut seen, Duration::from_millis(100));
        }
    }

    /// 백오프를 넘기고 디스패처를 깨운다
    pub fn skip_backoff(&self) {
        self.clock.advance(2 * 60 * 60 * 1000);
        self.queue.notify();
    }

    /// 지금 대기 중인 `jobs` 를 모두 끝낸다 (재등록되는 작업은 세지 않는다)
    pub fn run_pending_jobs(&self) -> usize {
        let n = self.jobs_rows().iter().filter(|j| j.status == "pending").count();
        if n > 0 {
            self.run(n);
        }
        n
    }
}
