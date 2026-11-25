//! P6 — `PluginRuntime`: 레지스트리·호스트(플러그인당 1개)·호출·번역. 설치·제거·부팅은 `install/`, 작업 대기열 연동은 `jobs/`(P5).
//!
//! 락 규칙: `registry` 는 말단 락(조회·교체만, 보유 중 호출·DB 금지). 호출 경로는 `Arc<PluginHost>` 를 복제해 락을 놓고 호출한다.
//! `lifecycle` 은 설치·제거·활성·권한·자동 비활성 쓰기를 직렬화한다(사용자 작업은 `try_lock` -> `Busy`).

mod autodisable;
mod contrib;
mod entry;
mod invoke;
pub(crate) mod l10n;
mod load;
mod locate;
mod manage;

pub(crate) use entry::{Entry, Pkg};
pub use locate::{WORKER_ENV, WORKER_EXE, locate_worker, locate_worker_default};
pub(crate) use manage::parse_granted_perms;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_db::{DbError, DbPool, PoolError, Priority};
use mam_plugin_gateway::{Gateway, GatewayConfig};
use mam_plugin_host::HostConfig;
use rusqlite::Connection;

use crate::api::{
    ChangeReason, ChangeSink, InstallError, JobHooks, NoopHooks, PluginChanged, PluginId,
};

/// 사용자 작업·부팅의 DB 대기 상한
const DB_WAIT: Duration = Duration::from_secs(30);

pub struct RuntimeDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub host_clock: Arc<dyn mam_plugin_host::Clock>,
    pub data_root: PathBuf,
    pub plugins_dir: PathBuf,
    pub worker_exe: Option<PathBuf>,
    pub app_version: semver::Version,
    pub gateway: GatewayConfig,
    pub on_change: ChangeSink,
}

pub type HostTweak = Arc<dyn Fn(&mut HostConfig) + Send + Sync>;

/// 시험 전용 조절 값 — 제품 코드는 설정하지 않는다
#[doc(hidden)]
#[derive(Default, Clone)]
pub struct TestKnobs {
    /// 제거 시 데이터 삭제 배치 크기 (기본 5,000)
    pub delete_batch: Option<usize>,
    /// 이후 새로 만드는 호스트 설정 조정 (호출 타임아웃·차단기 등)
    pub host_tweak: Option<HostTweak>,
}

pub struct PluginRuntime {
    pub(crate) deps: RuntimeDeps,
    pub(crate) gateway: Arc<Gateway>,
    pub(crate) registry: RwLock<BTreeMap<String, Arc<Entry>>>,
    pub(crate) lifecycle: Mutex<()>,
    pub(crate) hooks: RwLock<Arc<dyn JobHooks>>,
    pub(crate) knobs: RwLock<TestKnobs>,
    pub(crate) worker_available: bool,
}

impl PluginRuntime {
    /// 앱 시작을 실패시키지 않는다 — 깨진 플러그인은 목록에 `broken` 으로 남는다
    pub fn boot(deps: RuntimeDeps) -> Arc<Self> {
        let gateway = Arc::new(Gateway::new(
            Arc::clone(&deps.pool),
            Arc::clone(&deps.clock),
            deps.gateway.clone(),
        ));
        let worker_available = deps
            .worker_exe
            .as_deref()
            .is_some_and(std::path::Path::is_file);
        let rt = Arc::new(Self {
            deps,
            gateway,
            registry: RwLock::new(BTreeMap::new()),
            lifecycle: Mutex::new(()),
            hooks: RwLock::new(Arc::new(NoopHooks)),
            knobs: RwLock::new(TestKnobs::default()),
            worker_available,
        });
        crate::install::boot::load_all(&rt);
        rt
    }

    pub fn set_job_hooks(&self, hooks: Arc<dyn JobHooks>) {
        *self.hooks.write().unwrap_or_else(|p| p.into_inner()) = hooks;
    }

    #[doc(hidden)]
    pub fn set_test_knobs(&self, knobs: TestKnobs) {
        *self.knobs.write().unwrap_or_else(|p| p.into_inner()) = knobs;
    }

    /// 호스트 함수 게이트웨이 — `PluginJobs` 가 provider 반환값을 기록할 때 같은 권한 레지스트리·계량기를 쓰도록 공유한다 (P5 가 추가)
    pub fn gateway(&self) -> Arc<Gateway> {
        Arc::clone(&self.gateway)
    }

    /// 시험 전용 — 플러그인의 호스트(PID·시작 횟수 확인)
    #[doc(hidden)]
    pub fn host_for_test(&self, id: &str) -> Option<Arc<mam_plugin_host::PluginHost>> {
        self.entry(id).and_then(|e| e.host.clone())
    }

    /// 종료 시 JobRunner 정지 뒤에 부른다 — 모든 워커를 정리한다(진행 중 호출이 끝나길 기다린다)
    pub fn shutdown_all(&self) {
        let hosts: Vec<_> = self
            .registry
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter_map(|e| e.host.clone())
            .collect();
        for h in hosts {
            h.shutdown();
        }
    }

    // ---- 내부 도우미 ----

    pub(crate) fn entry(&self, id: &str) -> Option<Arc<Entry>> {
        self.registry
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    pub(crate) fn put_entry(&self, entry: Entry) -> Option<Arc<Entry>> {
        self.registry
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(entry.id.clone(), Arc::new(entry))
    }

    pub(crate) fn take_entry(&self, id: &str) -> Option<Arc<Entry>> {
        self.registry
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
    }

    pub(crate) fn emit(&self, id: &str, reason: ChangeReason) {
        (self.deps.on_change)(PluginChanged {
            plugin_id: id.to_owned(),
            reason,
        });
    }

    pub(crate) fn current_hooks(&self) -> Arc<dyn JobHooks> {
        Arc::clone(&self.hooks.read().unwrap_or_else(|p| p.into_inner()))
    }

    pub(crate) fn try_lifecycle(&self) -> Result<MutexGuard<'_, ()>, InstallError> {
        match self.lifecycle.try_lock() {
            Ok(g) => Ok(g),
            Err(TryLockError::Poisoned(p)) => Ok(p.into_inner()),
            Err(TryLockError::WouldBlock) => Err(InstallError::Busy),
        }
    }

    pub(crate) fn db_write<T, F>(&self, priority: Priority, f: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
    {
        self.deps
            .pool
            .write_until(priority, Instant::now() + DB_WAIT, f)
    }

    pub(crate) fn db_read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, PoolError> {
        self.deps.pool.read_until(Instant::now() + DB_WAIT, f)
    }

    /// 권한 레지스트리를 DB 기준으로 다시 채운다. 실패하면(닫힘 유지) 경고만 남긴다
    pub(crate) fn refresh_grants(&self, id: &PluginId) {
        if let Err(e) = self
            .gateway
            .grants()
            .refresh(&self.deps.pool, id, Instant::now() + DB_WAIT)
        {
            tracing::warn!(plugin = id.as_str(), error = %e, "grant refresh failed; plugin stays closed");
        }
    }
}
