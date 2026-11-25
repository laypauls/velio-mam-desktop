//! `PluginJobs` 내부 공유 상태 — 스레드(피드·전체 실행)와 실행기가 `Arc<Core>` 로 함께 쓴다. 조절 값·정지 신호·DB 도우미.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::jobs::JobQueue;
use mam_db::plugin_settings::{self, INSTALLED, PutError};
use mam_db::plugin_tasks::{RUN_ALL_BATCH, with_immediate_tx};
use mam_db::{DbError, DbPool, PoolError, Priority};
use mam_plugin_gateway::Gateway;
use rusqlite::{Connection, Transaction};

use crate::api::ProviderInvoker;

/// 피드 backpressure — 한 플러그인의 대기 작업이 이 수 이상이면 커서를 멈춘다 (제안값, 미측정)
pub const PENDING_CAP: i64 = 10_000;
/// 피드 한 번에 훑는 outbox 이벤트 id 폭 (M3 실측 후 2,000 -> 500: Low tx 1개가 쓰기기를 오래 잡아 스캔을 늦췄다)
pub const FEED_BATCH: i64 = 500;
/// 피드 리듬: 배치가 남았으면 짧게 쉬고 계속, 아니면 느리게 점검
pub const FEED_BUSY_PAUSE: Duration = Duration::from_millis(50);
pub const FEED_IDLE: Duration = Duration::from_secs(2);
/// 쓰기 대기 상한 (실행기·피드·전체 실행의 Low 쓰기)
pub const WRITE_WAIT: Duration = Duration::from_secs(30);

/// 시험 전용 조절 값 — 제품은 기본값만 쓴다 (`PluginJobs::set_tuning`)
#[derive(Debug, Clone)]
pub struct JobsTuning {
    pub pending_cap: i64,
    pub feed_batch: i64,
    pub feed_busy_pause: Duration,
    pub feed_idle: Duration,
    pub run_all_batch: i64,
    pub write_wait: Duration,
}

impl Default for JobsTuning {
    fn default() -> Self {
        Self {
            pending_cap: PENDING_CAP,
            feed_batch: FEED_BATCH,
            feed_busy_pause: FEED_BUSY_PAUSE,
            feed_idle: FEED_IDLE,
            run_all_batch: RUN_ALL_BATCH,
            write_wait: WRITE_WAIT,
        }
    }
}

/// 정지 신호 — 잠든 스레드를 즉시 깨운다
pub(super) struct Stopper {
    stopped: AtomicBool,
    lock: Mutex<()>,
    cv: Condvar,
}

impl Stopper {
    pub(super) fn new() -> Self {
        Self { stopped: AtomicBool::new(false), lock: Mutex::new(()), cv: Condvar::new() }
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.cv.notify_all();
    }

    /// `d` 동안 자거나 정지 신호에 깬다. 정지됐으면 `true`
    pub(super) fn sleep(&self, d: Duration) -> bool {
        let g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if self.is_stopped() {
            return true;
        }
        drop(self.cv.wait_timeout_while(g, d, |_| !self.is_stopped()).unwrap_or_else(|p| p.into_inner()));
        self.is_stopped()
    }
}

pub(crate) struct Core {
    pub(super) pool: Arc<DbPool>,
    pub(super) clock: SharedClock,
    pub(super) queue: Arc<JobQueue>,
    pub(super) invoker: Arc<dyn ProviderInvoker>,
    pub(super) gateway: Arc<Gateway>,
    protected: Vec<std::path::PathBuf>,
    /// 호출 때마다 현재 설정에서 보호 경로를 다시 계산한다 (실행 중 바뀌는 백업 폴더) — 고정 목록에 더해진다
    protected_source: RwLock<Option<ProtectedSource>>,
    pub(super) stopper: Stopper,
    tuning: RwLock<JobsTuning>,
}

/// 내보내기 금지 경로를 돌려주는 함수 (호출 시점의 설정 기준)
pub type ProtectedSource = Arc<dyn Fn() -> Vec<std::path::PathBuf> + Send + Sync>;

impl Core {
    pub(super) fn new(
        pool: Arc<DbPool>,
        clock: SharedClock,
        queue: Arc<JobQueue>,
        invoker: Arc<dyn ProviderInvoker>,
        gateway: Arc<Gateway>,
        protected: Vec<std::path::PathBuf>,
    ) -> Self {
        Self {
            pool,
            clock,
            queue,
            invoker,
            gateway,
            protected,
            protected_source: RwLock::new(None),
            stopper: Stopper::new(),
            tuning: RwLock::new(JobsTuning::default()),
        }
    }

    pub(super) fn set_protected_source(&self, source: ProtectedSource) {
        *self.protected_source.write().unwrap_or_else(|p| p.into_inner()) = Some(source);
    }

    /// 고정 보호 경로 + 지금 설정에서 계산한 보호 경로
    pub(super) fn protected_paths(&self) -> Vec<std::path::PathBuf> {
        let mut all = self.protected.clone();
        let source = self.protected_source.read().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(source) = source {
            all.extend(source());
        }
        all
    }

    pub(super) fn tuning(&self) -> JobsTuning {
        self.tuning.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(super) fn set_tuning(&self, t: JobsTuning) {
        *self.tuning.write().unwrap_or_else(|p| p.into_inner()) = t;
    }

    pub(super) fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    pub(super) fn deadline(&self) -> Instant {
        Instant::now() + self.tuning().write_wait
    }

    /// 정지 신호(`stop`) 또는 앱 종료
    pub(super) fn halted(&self) -> bool {
        self.stopper.is_stopped() || self.queue.is_shutdown()
    }

    pub(super) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T, DbError>) -> Result<T, PoolError> {
        self.pool.read_until(self.deadline(), f)
    }

    /// 설치 확인 + 한 Low 쓰기 트랜잭션. 플러그인이 제거됐으면(`plugin_installed` 없음) 아무것도 쓰지 않고 `None`.
    /// 제거는 설정 행을 먼저 지우므로 이 확인을 통과한 쓰기는 제거 뒤에 끼어들지 못한다.
    pub(super) fn write_installed<T, F>(&self, plugin: &str, f: F) -> Result<Option<T>, PoolError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>, i64) -> Result<T, DbError> + Send + 'static,
    {
        let (plugin, now) = (plugin.to_owned(), self.now());
        self.pool.write_until(Priority::Low, self.deadline(), move |c| {
            with_immediate_tx(c, |tx| {
                if !is_installed(tx, &plugin)? {
                    return Ok(None);
                }
                f(tx, now).map(Some)
            })
        })
    }
}

pub(super) fn is_installed(conn: &Connection, plugin: &str) -> rusqlite::Result<bool> {
    Ok(plugin_settings::get(conn, INSTALLED, plugin)?.is_some())
}

pub(super) fn put_err(e: PutError) -> DbError {
    match e {
        PutError::Db(e) => DbError::Sqlite(e),
        PutError::Id(e) => DbError::Io(std::io::Error::other(e.to_string())),
    }
}

/// 이 크기보다 큰 입력은 한 쓰기에 싣지 않는다 (hooks 의 대기 작업 재등록 배치)
pub(super) const REQUEUE_BATCH: i64 = 2_000;

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Concurrency(정지 신호가 잠든 스레드를 깨운다) / Boundary(이미 정지)

    #[test]
    fn t_pjob_core_01_stopper_wakes_sleeper_and_reports_stopped() {
        let s = Arc::new(Stopper::new());
        assert!(!s.sleep(Duration::from_millis(5)));
        let s2 = Arc::clone(&s);
        let h = std::thread::spawn(move || s2.sleep(Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(20));
        s.stop();
        assert!(h.join().unwrap());
        assert!(s.sleep(Duration::from_secs(30)), "already stopped returns immediately");
    }
}
