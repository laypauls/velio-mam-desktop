//! 측정 환경 — 실제 `DbPool`·`PluginRuntime`·`PluginJobs`·`UserData`·`QueryService` 를 `src-tauri/src/lib.rs` 와 같은 방식으로 묶는다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mam_core::SharedClock;
use mam_core::jobs::JobQueue;
use mam_core::query::QueryService;
use mam_core::userdata::{UpdateTagsInput, UserData};
use mam_db::{DbPool, Priority, SystemClock};
use mam_plugin_runtime::{GatewayConfig, HostSystemClock, PluginJobs, PluginJobsDeps, PluginRuntime, RuntimeDeps};

use crate::util::Rng;

pub const PLUGIN_A: &str = "com.example.integ";
pub const PLUGIN_B: &str = "com.example.integ2";
pub const PLUGIN_C: &str = "com.example.integ3";
pub const PROVIDER: &str = "meta.extract";

pub struct Env {
    pub pool: Arc<DbPool>,
    pub rt: Arc<PluginRuntime>,
    pub jobs: Arc<PluginJobs>,
    pub userdata: Arc<UserData>,
    pub query: Arc<QueryService>,
    pub root: PathBuf,
}

pub fn worker_from_env() -> Option<PathBuf> {
    std::env::var_os("MAM_PLUGIN_WORKER_EXE").map(PathBuf::from)
}

impl Env {
    /// `root/mam.sqlite` 이 이미 있는 데이터 루트에 풀·런타임을 연다. `queue` 가 없으면 독립 `JobQueue`(실행기 없음)를 만든다
    pub fn open(root: &Path, worker: Option<PathBuf>, queue: Option<Arc<JobQueue>>) -> Self {
        let pool = Arc::new(DbPool::open(&root.join("mam.sqlite"), 4).expect("open pool"));
        Self::with_pool(pool, root, worker, queue)
    }

    pub fn with_pool(pool: Arc<DbPool>, root: &Path, worker: Option<PathBuf>, queue: Option<Arc<JobQueue>>) -> Self {
        let clock: SharedClock = Arc::new(SystemClock);
        let queue = queue.unwrap_or_else(|| JobQueue::new(Arc::clone(&pool), Arc::clone(&clock)));
        let rt = PluginRuntime::boot(RuntimeDeps {
            pool: Arc::clone(&pool),
            clock: Arc::clone(&clock),
            host_clock: Arc::new(HostSystemClock),
            data_root: root.to_path_buf(),
            plugins_dir: root.join("plugins"),
            worker_exe: worker,
            app_version: semver::Version::new(0, 2, 0),
            gateway: GatewayConfig::new(root.join("cache")),
            on_change: Arc::new(|_| {}),
        });
        let jobs = PluginJobs::new(PluginJobsDeps {
            pool: Arc::clone(&pool),
            clock: Arc::clone(&clock),
            queue: Arc::clone(&queue),
            invoker: Arc::clone(&rt) as _,
            gateway: rt.gateway(),
            protected: vec![root.to_path_buf(), root.join("backups")],
        });
        rt.set_job_hooks(Arc::clone(&jobs) as _);
        let userdata = Arc::new(UserData::new(Arc::clone(&pool), Arc::clone(&clock)));
        let query = Arc::new(QueryService::new(Arc::clone(&pool), clock));
        Self { pool, rt, jobs, userdata, query, root: root.to_path_buf() }
    }

    /// 통합 시험 픽스처(`tests/fixtures/jobs/integ`, image provider `meta.extract`)를 `id` 로 바꿔 설치한다 (설치 훅이 피드 커서를 outbox 끝에 둔다)
    pub fn install(&self, id: &str) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("jobs").join("integ");
        // 플러그인 소스는 데이터 루트(MAMData) 밖에 둔다 — `inspect_dir` 가 겹침을 거부한다
        let dir = self.root.parent().unwrap_or(&self.root).join("plugin-src").join(id);
        std::fs::create_dir_all(&dir).expect("src dir");
        std::fs::copy(from.join("main.js"), dir.join("main.js")).expect("copy main");
        let manifest = std::fs::read_to_string(from.join("plugin.json")).expect("manifest").replace(PLUGIN_A, id);
        std::fs::write(dir.join("plugin.json"), manifest).expect("write manifest");
        let perms = ["asset.read.metadata", "kv.write", "tag.write"].map(String::from).to_vec();
        self.rt.install(&dir, id, "1.0.0", perms, "en").expect("install the fixture plugin");
    }

    /// 실제 사용자 태그 편집(High 쓰기) 클로저 — 자산 1..=`max` 중 무작위
    pub fn editor(&self, max: u64, seed: u64) -> impl FnMut() -> bool + Send + 'static {
        let ud = Arc::clone(&self.userdata);
        let mut rng = Rng(seed);
        let mut k = 0u64;
        move || {
            k += 1;
            let asset = i64::try_from(1 + rng.below(max)).unwrap_or(1);
            ud.update_asset_tags(UpdateTagsInput { asset_ids: vec![asset], add: vec![format!("bench-{}", k % 64)], remove: vec![] }).is_ok()
        }
    }

    /// 검색 결과 페이지 구성(`QueryService::hydrate` = `hydrate_page`, 읽기 연결) — 무작위 50개 자산
    pub fn reader(&self, max: u64, seed: u64) -> impl FnMut() -> bool + Send + 'static {
        let q = Arc::clone(&self.query);
        let mut rng = Rng(seed);
        move || {
            let ids: Vec<i64> = (0..50).map(|_| i64::try_from(1 + rng.below(max)).unwrap_or(1)).collect();
            q.hydrate(&ids).is_ok()
        }
    }

    pub fn count(&self, sql: &'static str) -> i64 {
        self.pool.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).expect("count")
    }

    /// 쓰기 연결에서 WAL 을 비운다 (읽기 연결이 열려 있어도 호출 시점에 읽는 트랜잭션이 없으면 TRUNCATE 된다)
    pub fn checkpoint(&self) {
        self.pool.write(Priority::High, |c| Ok(c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?)).expect("checkpoint");
    }

    pub fn shutdown(&self) {
        self.jobs.stop();
        self.rt.shutdown_all();
    }
}
