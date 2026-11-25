//! 작업 실행: 실행기 등록부 + 디스패처 스레드.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{Job, JobError, JobKind, JobOutcome, JobQueue};

/// 디스패처가 새 신호 없이 깨어나는 최대 간격 (`not_before` 도래 확인).
const IDLE_POLL: Duration = Duration::from_secs(1);

/// 실행기가 체크포인트에서 중단 여부를 묻는 창구.
pub struct JobContext<'a> {
    queue: &'a JobQueue,
    kind: JobKind,
    /// 이 작업만의 취소 플래그 (앱 종료도 켠다) - 디스크 꺼내기 준비 등이 특정 작업을 양보시킨다
    cancel: Arc<AtomicBool>,
}

impl JobContext<'_> {
    /// 일시정지(전역·해당 종류)·앱 종료·이 작업의 취소 요청이면 true — `Ok(JobOutcome::Yielded)` 로 빠져나온다.
    pub fn should_stop(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::SeqCst)
            || self.queue.is_shutdown()
            || self.queue.is_paused(self.kind)
    }

    /// 앱 종료·이 작업의 취소 신호 (긴 I/O 중 취소용). 일시정지는 포함하지 않는다
    pub fn shutdown_flag(&self) -> &AtomicBool {
        &self.cancel
    }
}

/// 작업 종류별 실행기. 블로킹 작업이므로 호출 스레드는 전용 실행 스레드다.
pub trait JobExecutor: Send + Sync {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError>;
}

#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    map: HashMap<JobKind, Arc<dyn JobExecutor>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 같은 종류의 실행기를 다시 등록하면 교체한다.
    pub fn register(&mut self, kind: JobKind, executor: Arc<dyn JobExecutor>) {
        self.map.insert(kind, executor);
    }

    pub fn kinds(&self) -> Vec<JobKind> {
        let mut k: Vec<_> = self.map.keys().copied().collect();
        k.sort();
        k
    }
}

pub struct JobRunner {
    queue: Arc<JobQueue>,
    dispatcher: Mutex<Option<JoinHandle<()>>>,
    workers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl JobRunner {
    pub fn start(queue: Arc<JobQueue>, registry: ExecutorRegistry) -> std::io::Result<Self> {
        let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::default();
        let handle = {
            let queue = Arc::clone(&queue);
            let workers = Arc::clone(&workers);
            std::thread::Builder::new()
                .name("mam-job-dispatch".into())
                .spawn(move || dispatch_loop(&queue, &registry, &workers))?
        };
        Ok(Self {
            queue,
            dispatcher: Mutex::new(Some(handle)),
            workers,
        })
    }

    /// 새 작업 시작을 멈추고, 실행 중인 작업이 체크포인트에서 `Yielded` 로 끝나길 기다린 뒤 스레드를 정리한다.
    pub fn stop(&self) {
        self.stop_within(Duration::MAX);
    }

    /// [`Self::stop`] 이되 `grace` 안에 끝나지 않는 실행 스레드는 기다리지 않고 떼어 둔다. 떼어 낸 수를 돌려준다.
    /// 떼어 낸 작업은 DB 에 `running` 으로 남고 다음 실행의 `JobQueue::recover` 가 시도 횟수 변화 없이 `pending` 으로 되돌린다.
    pub fn stop_within(&self, grace: Duration) -> usize {
        self.queue.request_shutdown();
        let started = std::time::Instant::now();
        // 디스패처를 먼저 정리한다 - 그래야 정리 도중 새 작업 스레드가 늘어나지 않는다
        let dispatcher: Vec<JoinHandle<()>> = self
            .dispatcher
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .into_iter()
            .collect();
        let mut abandoned = crate::join::join_within(dispatcher, grace);
        let workers: Vec<JoinHandle<()>> = self
            .workers
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        abandoned += crate::join::join_within(workers, grace.saturating_sub(started.elapsed()));
        if abandoned > 0 {
            tracing::warn!(
                abandoned,
                "job threads still running at shutdown; leaving them to the next start's recovery"
            );
        }
        abandoned
    }
}

fn dispatch_loop(
    queue: &Arc<JobQueue>,
    registry: &ExecutorRegistry,
    workers: &Arc<Mutex<Vec<JoinHandle<()>>>>,
) {
    let kinds = registry.kinds();
    let mut seen = 0u64;
    while !queue.is_shutdown() {
        match queue.claim_next(&kinds) {
            Ok(Some(job)) => {
                let Some(executor) = registry.map.get(&job.kind).map(Arc::clone) else {
                    continue;
                };
                let q = Arc::clone(queue);
                let spawned = std::thread::Builder::new()
                    .name(format!("mam-job-{}", job.kind.as_str()))
                    .spawn(move || run_one(&q, executor.as_ref(), &job));
                if let Ok(handle) = spawned
                    && let Ok(mut g) = workers.lock()
                {
                    g.retain(|h| !h.is_finished());
                    g.push(handle);
                }
            }
            Ok(None) => queue.wait(&mut seen, IDLE_POLL),
            Err(e) => {
                tracing::warn!(%e, "job claim failed");
                queue.wait(&mut seen, IDLE_POLL);
            }
        }
    }
}

/// 처리 작업 스레드를 낮은 우선순위로 돌린다 (D8: 검색·UI 반응이 메타·썸네일·해시·AI 보다 먼저). 작업마다 새 스레드라 되돌릴 필요가 없다.
/// 스캔은 파일 목록을 빨리 만들어야 하므로 제외한다.
#[cfg(windows)]
fn lower_thread_priority(kind: JobKind) {
    if kind != JobKind::Scan {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        // 실패해도 처리는 계속된다 (우선순위만 그대로)
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
    }
}

#[cfg(not(windows))]
fn lower_thread_priority(_kind: JobKind) {}

fn run_one(queue: &JobQueue, executor: &dyn JobExecutor, job: &Job) {
    lower_thread_priority(job.kind);
    let ctx = JobContext {
        queue,
        kind: job.kind,
        cancel: queue.job_cancel_flag(job.job_id),
    };
    let result = match catch_unwind(AssertUnwindSafe(|| executor.execute(job, &ctx))) {
        Ok(r) => r,
        Err(_) => Err(JobError::retryable("job executor panicked")),
    };
    if let Err(e) = &result {
        tracing::warn!(kind = job.kind.as_str(), job_id = job.job_id, error = %e.message, "job failed");
    }
    if let Err(e) = queue.finish(job, result) {
        tracing::error!(%e, job_id = job.job_id, "cannot record job result");
    }
}
