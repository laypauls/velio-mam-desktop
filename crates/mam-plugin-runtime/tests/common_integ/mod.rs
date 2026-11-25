//! P5 통합 시험 공용 — 실제 `PluginRuntime`(실제 워커) + `PluginJobs` 를 `src-tauri/src/lib.rs` 와 같은 방식으로 연결한다
//! (`PluginJobs::new({pool, clock, queue, invoker: runtime, gateway: runtime.gateway(), protected})` + `set_job_hooks`).
//! 사용자 편집은 실제 `UserData`, 작업은 실제 `JobRunner`. 대기는 채널·큐 condvar 뿐이다.
//! P6 의 `common_rt::Rt`(임시 폴더·DB·런타임)를 그대로 쓰고, 플러그인 소스는 `tests/fixtures/jobs/integ` 를 임시 폴더로 복사한다.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::jobs::{
    ExecutorRegistry, Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, JobQueue,
    JobRunner,
};
use mam_core::userdata::{UpdateTagsInput, UserData};
use mam_plugin_runtime::{
    Permission, PluginJobs, PluginJobsDeps, PluginRuntime, ProviderInvoker, ProviderSpec, RunError,
};
use serde_json::Value;

use crate::common_rt::{FixedClock, Rt, T0, copy_dir};

pub const ID: &str = "com.example.integ";
pub const PROVIDER: &str = "meta.extract";

/// 실제 런타임에 위임하되 호출이 시작될 때 신호를 보낸다 (진행 중 호출 시점을 결정적으로 알기 위한 얇은 감시자)
struct Spy {
    inner: Arc<PluginRuntime>,
    started: Mutex<Sender<()>>,
}

impl ProviderInvoker for Spy {
    fn auto_providers(&self) -> Vec<ProviderSpec> {
        self.inner.auto_providers()
    }

    fn provider(&self, plugin: &str, provider: &str) -> Result<ProviderSpec, RunError> {
        self.inner.provider(plugin, provider)
    }

    fn allows(&self, plugin: &str, perm: Permission) -> bool {
        self.inner.allows(plugin, perm)
    }

    fn invoke_provider(
        &self,
        plugin: &str,
        provider: &str,
        assets: Vec<i64>,
        params: Value,
    ) -> Result<Value, RunError> {
        let _ = self.started.lock().unwrap().send(());
        self.inner.invoke_provider(plugin, provider, assets, params)
    }
}

/// 끝난 작업 1건의 결과를 채널로 알리는 실행기 감싸개
struct Probe {
    inner: Arc<dyn JobExecutor>,
    tx: Mutex<Sender<Result<JobOutcome, JobError>>>,
}

impl JobExecutor for Probe {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let outcome = self.inner.execute(job, ctx);
        let _ = self.tx.lock().unwrap().send(outcome.clone());
        outcome
    }
}

pub struct Integ {
    pub rt: Rt,
    pub jobs: Arc<PluginJobs>,
    pub queue: Arc<JobQueue>,
    userdata: Arc<UserData>,
    runner: Mutex<Option<JobRunner>>,
    done_tx: Sender<Result<JobOutcome, JobError>>,
    done_rx: Mutex<Receiver<Result<JobOutcome, JobError>>>,
    /// `with_spy` 일 때만: provider 호출이 시작될 때마다 신호
    pub started: Option<Receiver<()>>,
}

impl Integ {
    pub fn new() -> Self {
        Self::build(false)
    }

    pub fn with_spy() -> Self {
        Self::build(true)
    }

    fn build(spy: bool) -> Self {
        let rt = Rt::new();
        let clock: SharedClock = Arc::new(FixedClock(T0));
        let queue = JobQueue::new(Arc::clone(&rt.pool), Arc::clone(&clock));
        let (tx, rx) = channel();
        let invoker: Arc<dyn ProviderInvoker> = if spy {
            Arc::new(Spy {
                inner: Arc::clone(&rt.rt),
                started: Mutex::new(tx),
            })
        } else {
            Arc::clone(&rt.rt) as _
        };
        let jobs = PluginJobs::new(PluginJobsDeps {
            pool: Arc::clone(&rt.pool),
            clock: Arc::clone(&clock),
            queue: Arc::clone(&queue),
            invoker,
            gateway: rt.rt.gateway(),
            protected: vec![rt.data_root.clone(), rt.data_root.join("backups")],
        });
        rt.rt.set_job_hooks(Arc::clone(&jobs) as _);
        let userdata = Arc::new(UserData::new(Arc::clone(&rt.pool), clock));
        let (done_tx, done_rx) = channel();
        Self {
            rt,
            jobs,
            queue,
            userdata,
            runner: Mutex::new(None),
            done_tx,
            done_rx: Mutex::new(done_rx),
            started: spy.then_some(rx),
        }
    }

    /// 픽스처 플러그인을 임시 폴더로 복사해 `kv.write`·`tag.write`·`asset.read.metadata` 를 허용하고 설치한다
    pub fn install(&self) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("jobs")
            .join("integ");
        let dir: PathBuf = self.rt.src_root.join("integ");
        copy_dir(&from, &dir);
        let perms = ["asset.read.metadata", "kv.write", "tag.write"]
            .map(String::from)
            .to_vec();
        self.rt
            .rt
            .install(&dir, ID, "1.0.0", perms, "en")
            .expect("install the integration fixture");
    }

    /// 사용자 태그 편집 (새 태그 이름 `tag`) -> revision +1 과 outbox upsert 이벤트
    pub fn edit(&self, asset: i64, tag: &str) {
        self.userdata
            .update_asset_tags(UpdateTagsInput {
                asset_ids: vec![asset],
                add: vec![tag.to_owned()],
                remove: vec![],
            })
            .unwrap();
    }

    pub fn start_runner(&self) {
        let mut slot = self.runner.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let mut reg = ExecutorRegistry::new();
        reg.register(
            JobKind::Plugin,
            Arc::new(Probe {
                inner: self.jobs.executor(),
                tx: Mutex::new(self.done_tx.clone()),
            }),
        );
        *slot = Some(JobRunner::start(Arc::clone(&self.queue), reg).unwrap());
    }

    pub fn stop_runner(&self) {
        if let Some(r) = self.runner.lock().unwrap().take() {
            r.stop();
        }
    }

    /// 작업 `n` 건이 끝날 때까지 기다린다 (결과 채널) -> `finish` 의 DB 쓰기까지 마친 뒤 돌려준다
    pub fn run(&self, n: usize) -> Vec<Result<JobOutcome, JobError>> {
        self.start_runner();
        let out: Vec<_> = (0..n)
            .map(|_| {
                self.done_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(60))
                    .expect("a plugin job should finish within 60s")
            })
            .collect();
        self.settle();
        out
    }

    pub fn settle(&self) {
        let (limit, mut seen) = (Instant::now() + Duration::from_secs(30), 0u64);
        while self.queue.running_count(JobKind::Plugin) > 0 {
            assert!(Instant::now() < limit, "plugin job did not settle");
            self.queue.wait(&mut seen, Duration::from_millis(100));
        }
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.rt.count(sql)
    }

    pub fn kv(&self, asset: i64, key: &str) -> Option<String> {
        let key = key.to_owned();
        self.rt.pool.read(move |c| Ok(c.query_row("SELECT value FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = ?1 AND asset_id = ?2 AND key = ?3", (ID, asset, key), |r| r.get(0)).ok())).unwrap()
    }

    pub fn tags(&self, asset: i64) -> Vec<String> {
        self.rt
            .pool
            .read(move |c| {
                let mut st = c.prepare("SELECT t.name FROM asset_tags a JOIN tags t ON t.tag_id = a.tag_id WHERE a.asset_id = ?1 ORDER BY t.name")?;
                let rows = st.query_map([asset], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap()
    }

    /// `(status, target, done)`
    pub fn task(&self, asset: i64) -> Option<(String, i64, i64)> {
        self.rt
            .pool
            .read(move |c| Ok(c.query_row("SELECT t.status, t.target_revision, t.done_revision FROM plugin_tasks t JOIN plugins p USING (plugin_ref) JOIN plugin_providers v USING (provider_ref) WHERE p.plugin_id = ?1 AND v.name = ?2 AND t.asset_id = ?3", (ID, PROVIDER, asset), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()))
            .unwrap()
    }

    pub fn cursor(&self) -> Option<i64> {
        self.rt
            .pool
            .read(|c| Ok(mam_db::plugin_tasks::feed_cursor(c, ID)?))
            .unwrap()
    }
}

impl Drop for Integ {
    fn drop(&mut self) {
        self.stop_runner();
        self.jobs.stop();
        self.rt.rt.shutdown_all();
    }
}

impl Integ {
    /// 다른 스레드로 옮길 수 있는 편집 클로저 (자산, 새 태그 이름)
    pub fn editor(&self) -> impl Fn(i64, &str) + Send + Sync + 'static {
        let userdata = Arc::clone(&self.userdata);
        move |asset, tag| {
            userdata
                .update_asset_tags(UpdateTagsInput {
                    asset_ids: vec![asset],
                    add: vec![tag.to_owned()],
                    remove: vec![],
                })
                .unwrap();
        }
    }
}

impl Integ {
    /// 큐 condvar 로만 기다린다 (sleep 폴링 없음). 30 초 안에 조건이 안 되면 실패
    pub fn wait_until(&self, what: &str, cond: impl Fn(&Integ) -> bool) {
        let (limit, mut seen) = (Instant::now() + Duration::from_secs(30), 0u64);
        while !cond(self) {
            assert!(Instant::now() < limit, "timed out waiting for: {what}");
            self.queue.wait(&mut seen, Duration::from_millis(100));
        }
    }
}
