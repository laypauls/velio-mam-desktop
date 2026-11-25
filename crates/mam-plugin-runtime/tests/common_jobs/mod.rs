//! P5 시험 공용 — 실제 SQLite(마이그레이션 적용)·실제 `Gateway`·실제 워커 `PluginHost`·실제 mam-core `JobQueue`/`JobRunner`.
//! 원본 미디어 파일은 임시 폴더(`lib`)에만 만든다. 시계는 시험이 전진시키는 `TestClock`. 사용자 태그 편집은 실제 `UserData` 서비스를
//! 거쳐 revision·`search_outbox` 이벤트가 진짜로 생긴다. 플러그인 소스는 `tests/fixtures/jobs/*.js`.
//! 대기: `Probe` 채널과 큐의 condvar(`JobQueue::wait`)만 쓴다 — sleep 폴링 없음.
#![allow(dead_code)]

mod export;
mod invoker;
mod plugins;
mod queries;
mod runner;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use mam_core::SharedClock;
use mam_core::jobs::{JobQueue, JobRunner};
use mam_core::userdata::{UpdateTagsInput, UserData};
use mam_db::{DbPool, Priority, migrate, open};
use mam_plugin_gateway::{Gateway, GatewayConfig};
use mam_plugin_runtime::{PluginJobs, PluginJobsDeps};
use tempfile::TempDir;

#[allow(unused_imports)]
pub use export::*;
#[allow(unused_imports)]
pub use invoker::{Call, TestApi, TestInvoker};
#[allow(unused_imports)]
pub use plugins::*;
#[allow(unused_imports)]
pub use queries::*;
#[allow(unused_imports)]
pub use runner::Done;

pub const T0: i64 = 1_700_000_000_000;

/// 수동 전진 시계 — 작업 대기열(백오프 `not_before`)·게이트웨이·사용자 편집이 같은 시계를 쓴다
pub struct TestClock(AtomicI64);

impl TestClock {
    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl mam_db::Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// `search_outbox` 에서 작업으로 가는 길 전체를 담은 환경
pub struct Env {
    pub pool: Arc<DbPool>,
    pub clock: Arc<TestClock>,
    pub gw: Arc<Gateway>,
    pub invoker: Arc<TestInvoker>,
    pub queue: Arc<JobQueue>,
    pub jobs: Arc<PluginJobs>,
    pub userdata: Arc<UserData>,
    /// 원본 미디어가 있는 임시 폴더
    pub lib: PathBuf,
    vol: i64,
    /// 내보내기 금지 경로 (MAMData 루트·백업 폴더 — `src-tauri` 와 같은 구성)
    pub data: PathBuf,
    pub backups: PathBuf,
    edits: Arc<AtomicUsize>,
    runner: Mutex<Option<JobRunner>>,
    done_tx: Sender<Done>,
    done_rx: Mutex<Receiver<Done>>,
    /// 마지막에 해제된다 (풀·DB 파일이 닫힌 뒤 임시 폴더 삭제)
    pub tmp: TempDir,
}

/// 사용자 태그 편집 (스레드·훅에서 쓴다)
#[derive(Clone)]
pub struct Editor {
    userdata: Arc<UserData>,
    edits: Arc<AtomicUsize>,
    pool: Arc<DbPool>,
}

impl Editor {
    /// 새 태그를 붙인다 → revision +1 과 outbox upsert 이벤트. 새 revision 을 돌려준다
    pub fn edit(&self, asset: i64) -> i64 {
        let n = self.edits.fetch_add(1, Ordering::SeqCst);
        self.userdata
            .update_asset_tags(UpdateTagsInput {
                asset_ids: vec![asset],
                add: vec![format!("user-tag-{n}")],
                remove: vec![],
            })
            .unwrap();
        self.pool
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT revision FROM media_assets WHERE asset_id = ?1",
                    [asset],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }
}

impl Env {
    pub fn new() -> Self {
        Self::with(|_| {})
    }

    pub fn with(tweak: impl FnOnce(&mut GatewayConfig)) -> Self {
        let tmp = TempDir::new().unwrap();
        let (data, lib, backups) = (
            tmp.path().join("MAMData"),
            tmp.path().join("SecretLibrary"),
            tmp.path().join("Backups"),
        );
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        let clock = Arc::new(TestClock(AtomicI64::new(T0)));
        let shared: SharedClock = clock.clone();
        let db_path = data.join("mam.sqlite");
        let mut conn = open(&db_path).unwrap();
        migrate(&mut conn, &data.join("backups"), clock.as_ref()).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db_path, 2).unwrap());
        let vol = pool
            .write(Priority::High, |c| {
                c.execute("INSERT INTO volumes (kind, volume_key, label, last_mount_point, last_seen_at) VALUES ('fixed', 'vol-x', 'Vol', 'Z:\\Mount', 1)", [])?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();
        let mut cfg = GatewayConfig::new(data.join("cache"));
        tweak(&mut cfg);
        let gw = Arc::new(Gateway::new(Arc::clone(&pool), Arc::clone(&shared), cfg));
        let invoker = Arc::new(TestInvoker::new(
            Arc::clone(&gw),
            Arc::new(TestApi::new(Arc::clone(&gw))),
        ));
        let queue = JobQueue::new(Arc::clone(&pool), Arc::clone(&shared));
        let protected = vec![data.clone(), backups.clone()];
        let jobs = Self::make_jobs(&pool, &shared, &queue, &invoker, &gw, &protected);
        let userdata = Arc::new(UserData::new(Arc::clone(&pool), shared));
        let (done_tx, done_rx) = channel();
        Self {
            tmp,
            pool,
            clock,
            gw,
            invoker,
            queue,
            jobs,
            userdata,
            lib,
            vol,
            data,
            backups,
            edits: Arc::new(AtomicUsize::new(0)),
            runner: Mutex::new(None),
            done_tx,
            done_rx: Mutex::new(done_rx),
        }
    }

    fn make_jobs(
        pool: &Arc<DbPool>,
        clock: &SharedClock,
        queue: &Arc<JobQueue>,
        invoker: &Arc<TestInvoker>,
        gw: &Arc<Gateway>,
        protected: &[PathBuf],
    ) -> Arc<PluginJobs> {
        PluginJobs::new(PluginJobsDeps {
            pool: Arc::clone(pool),
            clock: Arc::clone(clock),
            queue: Arc::clone(queue),
            invoker: Arc::clone(invoker) as Arc<dyn mam_plugin_runtime::ProviderInvoker>,
            gateway: Arc::clone(gw),
            protected: protected.to_vec(),
        })
    }

    /// 앱 재시작 — 러너·대기열·`PluginJobs` 를 버리고 같은 DB 위에 새로 만든다 (`recover` 로 running 복구)
    pub fn restart(&mut self) {
        self.stop_runner();
        self.jobs.stop();
        let shared: SharedClock = self.clock.clone();
        self.queue = JobQueue::new(Arc::clone(&self.pool), Arc::clone(&shared));
        self.queue.recover().unwrap();
        let protected = vec![self.data.clone(), self.backups.clone()];
        self.jobs = Self::make_jobs(
            &self.pool,
            &shared,
            &self.queue,
            &self.invoker,
            &self.gw,
            &protected,
        );
    }

    // ── 자산 ────────────────────────────────────────────────

    /// 임시 폴더에 원본 파일을 만들고 활성 자산 + 위치를 등록한다 (이벤트 없음 — revision 1)
    pub fn add_asset(&self, media: &str, name: &str) -> i64 {
        let path = self.lib.join(name);
        std::fs::write(&path, format!("original bytes of {name}")).unwrap();
        let (media, name, vol) = (media.to_owned(), name.to_owned(), self.vol);
        self.pool
            .write(Priority::High, move |c| {
                c.execute("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES (?1, 1, 1, 1, 1)", [&media])?;
                let id = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, ?2, ?3, ?4, 1)",
                    (id, vol, &name, mam_db::path_key(&name).unwrap()),
                )?;
                Ok(id)
            })
            .unwrap()
    }

    /// 실제 `UserData` 로 새 태그를 붙인다 → revision +1 과 `search_outbox` upsert 이벤트 (사용자 편집). 새 revision 을 돌려준다
    pub fn edit(&self, asset: i64) -> i64 {
        self.editor().edit(asset)
    }

    /// 훅 클로저로 옮길 수 있는 편집기 (같은 `UserData`·같은 태그 이름 카운터)
    pub fn editor(&self) -> Editor {
        Editor {
            userdata: Arc::clone(&self.userdata),
            edits: Arc::clone(&self.edits),
            pool: Arc::clone(&self.pool),
        }
    }

    pub fn set_status(&self, asset: i64, status: &str) {
        self.exec(&format!(
            "UPDATE media_assets SET status = '{status}' WHERE asset_id = {asset}"
        ));
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.stop_runner();
        self.jobs.stop();
        self.invoker.shutdown_all();
    }
}

/// 폴더 아래 모든 파일의 (상대 경로, 내용, 수정 시각) — 원본 보호 단언용
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    let mut out: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let m = std::fs::metadata(&p).unwrap();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
                m.modified().unwrap(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// PID 로만 확인 — 이미지 이름으로 찾지 않는다
pub fn pid_alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .expect("tasklist");
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}
