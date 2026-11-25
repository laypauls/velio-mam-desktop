//! P5 — `PluginJobs`: 변경 피드·실행기·전체 실행·내보내기·작업 건수. 명세 §B.
//!
//! 모든 쓰기는 `Priority::Low` 한 트랜잭션이고 `plugin_installed` 를 먼저 확인한다 (제거는 설정 행을 먼저 지운다).
//! `jobs` 행 하나 = 자산 하나 — 실행기가 그 자산의 모든 플러그인 대기 작업을 순서대로 처리한다.

mod core;
mod executor;
mod export;
mod export_fs;
mod feed;
mod hooks;
mod run_all;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use mam_core::SharedClock;
use mam_core::jobs::{JobExecutor, JobQueue};
use mam_db::plugin_settings::{self, INSTALLED};
use mam_db::plugin_tasks;
use mam_db::{DbPool, PoolError};
use mam_plugin_gateway::Gateway;

use self::core::Core;
use crate::api::{ExportResult, PluginTaskCount, ProviderInvoker, RunAllResult};

pub use self::core::{
    FEED_BATCH, FEED_BUSY_PAUSE, FEED_IDLE, JobsTuning, PENDING_CAP, ProtectedSource, WRITE_WAIT,
};
pub use self::export::{ExportError, MAX_EXPORT_ASSETS, MAX_EXPORT_BYTES};
pub use self::run_all::RunAllError;

pub struct PluginJobsDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    /// `ingest.jobs()`
    pub queue: Arc<JobQueue>,
    /// `PluginRuntime`
    pub invoker: Arc<dyn ProviderInvoker>,
    /// `PluginRuntime::gateway()` — provider 반환값을 기록하고 작업을 완료한다 (`record_provider_output`)
    pub gateway: Arc<Gateway>,
    /// 내보내기 대상으로 금지하는 경로 (MAMData 루트·백업 폴더). 관리 폴더 루트는 실행 시 DB 에서 읽는다
    pub protected: Vec<PathBuf>,
}

/// `retry_failed` 오류
#[derive(Debug, thiserror::Error)]
pub enum PluginJobsError {
    #[error("plugin not installed")]
    NotInstalled,
    #[error(transparent)]
    Db(#[from] PoolError),
}

pub struct PluginJobs {
    core: Arc<Core>,
    feed: Mutex<Option<JoinHandle<()>>>,
    run_all: Arc<run_all::Registry>,
}

impl PluginJobs {
    pub fn new(deps: PluginJobsDeps) -> Arc<Self> {
        let core = Arc::new(Core::new(
            deps.pool,
            deps.clock,
            deps.queue,
            deps.invoker,
            deps.gateway,
            deps.protected,
        ));
        Arc::new(Self {
            core,
            feed: Mutex::new(None),
            run_all: Arc::new(run_all::Registry::default()),
        })
    }

    pub fn executor(&self) -> Arc<dyn JobExecutor> {
        Arc::new(executor::PluginExecutor::new(Arc::clone(&self.core)))
    }

    /// 변경 피드 스레드(`mam-plugin-feed`)를 시작한다 — `Ingest::start` 뒤에 부른다. 이미 시작했거나 정지했으면 아무 일도 하지 않는다
    pub fn start_feed(&self) {
        let mut slot = self.feed.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() || self.core.stopper.is_stopped() {
            return;
        }
        match feed::spawn(Arc::clone(&self.core)) {
            Ok(h) => *slot = Some(h),
            Err(e) => tracing::warn!(error = %e, "cannot start the plugin feed thread"),
        }
    }

    /// 피드·전체 실행 스레드를 멈추고 기다린다 (종료 시, 런타임 `shutdown_all` 보다 먼저). 여러 번 불러도 된다
    pub fn stop(&self) {
        self.core.stopper.stop();
        let feed = self.feed.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(h) = feed {
            let _ = h.join();
        }
        self.run_all.join_all();
    }

    /// provider 하나를 모든 활성 자산에 실행하도록 등록한다 (비동기 — 배치마다 작업 등록). `started=false` = 이미 실행 중이거나 종료 중
    pub fn run_all(&self, plugin: &str, provider: &str) -> Result<RunAllResult, RunAllError> {
        run_all::start(&self.core, &self.run_all, plugin, provider)
    }

    /// 내보내기 provider 를 직접 호출해 결과를 `dest_path` 에 원자적으로 쓴다 (블로킹 — `spawn_blocking` 에서)
    pub fn export(
        &self,
        plugin: &str,
        provider: &str,
        asset_ids: Vec<i64>,
        dest_path: &str,
    ) -> Result<ExportResult, ExportError> {
        export::export(&self.core, plugin, provider, asset_ids, dest_path)
    }

    /// 내보내기 금지 경로를 호출 때마다 현재 설정에서 다시 계산하는 함수를 연결한다 (고정 `protected` 에 더해진다)
    pub fn set_protected_source(&self, source: ProtectedSource) {
        self.core.set_protected_source(source);
    }

    /// 설치된 플러그인별 대기·실패 작업 수 (플러그인 ID 순)
    pub fn task_counts(&self) -> Result<Vec<PluginTaskCount>, PoolError> {
        let rows = self.core.read(|c| {
            let ids: Vec<String> = plugin_settings::load_prefixed(c, INSTALLED)?
                .into_keys()
                .collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            Ok(plugin_tasks::counts_by_plugin(c, &refs)?)
        })?;
        Ok(rows
            .into_iter()
            .map(|r| PluginTaskCount {
                plugin_id: r.plugin_id,
                pending: r.pending.max(0) as u64,
                failed: r.failed.max(0) as u64,
            })
            .collect())
    }

    /// `failed` 작업을 다시 대기로 돌리고 `jobs` 에 올린다. 되돌린 자산 수(중복 제거)를 돌려준다
    pub fn retry_failed(&self, plugin: &str) -> Result<u64, PluginJobsError> {
        let priority = mam_core::jobs::JobKind::Plugin.default_priority();
        let id = plugin.to_owned();
        let done = self.core.write_installed(plugin, move |tx, now| {
            let ids = plugin_tasks::retry_failed(tx, &id, now)?;
            plugin_tasks::enqueue_jobs_tx(tx, &ids, priority, now)?;
            Ok(ids.len())
        })?;
        let n = done.ok_or(PluginJobsError::NotInstalled)?;
        if n > 0 {
            self.core.queue.notify();
        }
        Ok(n as u64)
    }

    /// 시험 전용 — 조절 값 교체 (피드 간격·상한·배치 크기)
    #[doc(hidden)]
    pub fn set_tuning(&self, t: JobsTuning) {
        self.core.set_tuning(t);
    }

    /// 시험 전용 — 피드를 한 번 돈다 (스레드 없이 결정적으로). 배치가 더 남았으면 `true`
    #[doc(hidden)]
    pub fn feed_tick(&self) -> bool {
        feed::tick(&self.core, &self.core.tuning())
    }
}
