//! P6 시험 공용 — 실제 SQLite(마이그레이션 적용)·실제 워커·임시 폴더 픽스처·고정 시계·기록용 훅/알림 sink.
//! 플러그인 소스 폴더는 `tests/fixtures/plugins/<name>` 을 임시 폴더로 복사해 쓴다(실제 사용자 폴더 접근 없음).
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use mam_db::{DbPool, Priority, migrate, open};
use mam_plugin_host::Clock;
use mam_plugin_runtime::{ChangeReason, GatewayConfig, JobHooks, PluginChanged, PluginId, PluginInfo, PluginRuntime, RuntimeDeps};
use semver::Version;
use tempfile::TempDir;

pub const T0: i64 = 1_700_000_000_000;
pub const EN: &str = "en";

pub struct FixedClock(pub i64);

impl mam_db::Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// 호스트 차단기용 고정 시계 — 모든 결함이 같은 순간에 일어난 것으로 본다(창 안)
pub struct FixedHostClock(Instant);

impl FixedHostClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Instant::now()))
    }
}

impl Clock for FixedHostClock {
    fn now(&self) -> Instant {
        self.0
    }
}

/// 워커 바이너리 — 시험 프로세스 안에서 한 번 `cargo build -p mam-plugin-worker` (이미 최신이면 즉시 끝난다)
pub fn worker_exe() -> PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        if let Some(p) = std::env::var_os("MAM_PLUGIN_WORKER_EXE") {
            return PathBuf::from(p);
        }
        let test_exe = std::env::current_exe().expect("test exe path");
        let profile_dir = test_exe.parent().and_then(Path::parent).expect("profile dir").to_path_buf();
        let mut cmd = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args(["build", "-p", "mam-plugin-worker", "--quiet"]);
        if profile_dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        assert!(cmd.status().expect("run cargo build for the worker").success(), "building mam-plugin-worker failed");
        let exe = profile_dir.join("mam-plugin-worker.exe");
        assert!(exe.exists(), "worker not found at {}", exe.display());
        exe
    })
    .clone()
}

#[derive(Default)]
pub struct RecHooks {
    pub installed: Mutex<Vec<(String, bool)>>,
    pub enabled: Mutex<Vec<String>>,
}

impl JobHooks for RecHooks {
    fn installed(&self, plugin: &PluginId, fresh: bool) {
        self.installed.lock().unwrap().push((plugin.to_string(), fresh));
    }

    fn enabled(&self, plugin: &PluginId) {
        self.enabled.lock().unwrap().push(plugin.to_string());
    }
}

pub struct Rt {
    pub tmp: TempDir,
    pub pool: Arc<DbPool>,
    pub data_root: PathBuf,
    pub plugins_dir: PathBuf,
    pub src_root: PathBuf,
    pub rt: Arc<PluginRuntime>,
    pub events: Arc<Mutex<Vec<PluginChanged>>>,
    pub hooks: Arc<RecHooks>,
    vol: i64,
}

fn deps(pool: &Arc<DbPool>, data_root: &Path, plugins_dir: &Path, worker: Option<PathBuf>, app: &str, events: &Arc<Mutex<Vec<PluginChanged>>>) -> RuntimeDeps {
    let sink = Arc::clone(events);
    RuntimeDeps {
        pool: Arc::clone(pool),
        clock: Arc::new(FixedClock(T0)),
        host_clock: FixedHostClock::new(),
        data_root: data_root.to_path_buf(),
        plugins_dir: plugins_dir.to_path_buf(),
        worker_exe: worker,
        app_version: Version::parse(app).unwrap(),
        gateway: GatewayConfig::new(data_root.join("cache")),
        on_change: Arc::new(move |c| sink.lock().unwrap().push(c)),
    }
}

impl Rt {
    /// 실제 워커, 앱 버전 0.2.0
    pub fn new() -> Self {
        Self::with(Some(worker_exe()), "0.2.0")
    }

    pub fn with(worker: Option<PathBuf>, app: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let (data_root, src_root) = (tmp.path().join("MAMData"), tmp.path().join("src"));
        let plugins_dir = data_root.join("plugins");
        fs::create_dir_all(&src_root).unwrap();
        fs::create_dir_all(&data_root).unwrap();
        let db_path = data_root.join("mam.sqlite");
        let mut conn = open(&db_path).unwrap();
        migrate(&mut conn, &data_root.join("backups"), &FixedClock(T0)).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db_path, 2).unwrap());
        let vol = pool
            .write(Priority::High, |c| {
                c.execute("INSERT INTO volumes (kind, volume_key, label, last_mount_point, last_seen_at) VALUES ('fixed', 'vol-x', 'Vol', 'Z:\\Mount', 1)", [])?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let rt = PluginRuntime::boot(deps(&pool, &data_root, &plugins_dir, worker, app, &events));
        let hooks = Arc::new(RecHooks::default());
        rt.set_job_hooks(hooks.clone());
        Self { tmp, pool, data_root, plugins_dir, src_root, rt, events, hooks, vol }
    }

    /// 같은 DB·폴더로 런타임을 다시 부팅한다 (앱 재시작). 이전 런타임의 워커는 정리한다
    pub fn reboot(&mut self, worker: Option<PathBuf>, app: &str) {
        self.rt.shutdown_all();
        self.rt = PluginRuntime::boot(deps(&self.pool, &self.data_root, &self.plugins_dir, worker, app, &self.events));
        self.rt.set_job_hooks(self.hooks.clone());
    }

    /// 픽스처 플러그인을 `src/<dest>` 로 복사한다
    pub fn fixture(&self, name: &str, dest: &str) -> PathBuf {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("plugins").join(name);
        let to = self.src_root.join(dest);
        copy_dir(&from, &to);
        to
    }

    /// 파일 내용을 바꾼다 (`f(원문) -> 새 원문`)
    pub fn patch(&self, dir: &Path, rel: &str, f: impl FnOnce(&str) -> String) {
        let path = dir.join(rel);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, f(&text)).unwrap();
    }

    /// 선언한 권한 전부를 허용해 설치한다
    pub fn install_basic(&self, dir: &Path) -> PluginInfo {
        self.rt.install(dir, "com.example.basic", &self.version_of(dir), basic_perms(), EN).unwrap()
    }

    pub fn version_of(&self, dir: &Path) -> String {
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("plugin.json")).unwrap()).unwrap();
        v["version"].as_str().unwrap().to_owned()
    }

    pub fn events(&self) -> Vec<(String, ChangeReason)> {
        self.events.lock().unwrap().iter().map(|c| (c.plugin_id.clone(), c.reason)).collect()
    }

    pub fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.pool.write(Priority::High, move |c| Ok(c.execute_batch(&sql)?)).unwrap();
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.pool.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        let key = key.to_owned();
        self.pool
            .read(move |c| Ok(c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get::<_, String>(0)).ok()))
            .unwrap()
    }

    /// 이미지 자산 1건 (활성 위치 포함) — 원본 파일 이름이 `asset.get` 의 `fileName` 이다
    pub fn add_asset(&self, rel: &str) -> i64 {
        let (rel, vol) = (rel.to_owned(), self.vol);
        self.pool
            .write(Priority::High, move |c| {
                c.execute("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES ('image', 1, 1, 1, 1)", [])?;
                let id = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, ?2, ?3, ?4, 1)",
                    (id, vol, &rel, mam_db::path_key(&rel).unwrap()),
                )?;
                Ok(id)
            })
            .unwrap()
    }

    /// 플러그인 전역 KV 행 `n` 개 + (있으면) 자산 작업 행
    pub fn seed_data(&self, plugin: &str, n: usize, task_assets: &[i64]) {
        let (p, n) = (plugin.to_owned(), n as i64);
        let assets = task_assets.to_vec();
        self.pool
            .write(Priority::High, move |c| {
                c.execute("INSERT OR IGNORE INTO plugins (plugin_id) VALUES (?1)", [&p])?;
                c.execute("INSERT OR IGNORE INTO plugin_providers (plugin_ref, name) SELECT plugin_ref, 'meta.extract' FROM plugins WHERE plugin_id = ?1", [&p])?;
                for i in 0..n {
                    c.execute("INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) SELECT plugin_ref, NULL, ?2, '1', 1 FROM plugins WHERE plugin_id = ?1", (&p, format!("k{i}")))?;
                }
                for a in assets {
                    c.execute("INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at) SELECT p.plugin_ref, v.provider_ref, ?2, 1, 1 FROM plugins p JOIN plugin_providers v USING (plugin_ref) WHERE p.plugin_id = ?1 AND v.name = 'meta.extract'", (&p, a))?;
                }
                Ok(())
            })
            .unwrap();
    }
}

pub fn basic_perms() -> Vec<String> {
    ["asset.read.metadata", "kv.read", "kv.write", "tag.write", "export.write"].iter().map(|s| (*s).to_owned()).collect()
}

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            fs::copy(e.path(), target).unwrap();
        }
    }
}

/// 폴더 아래 모든 파일의 (상대 경로, 내용, 수정 시각) — 원본 보호 단언용
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>, SystemTime)> {
    let mut out = Vec::new();
    walk(root, &mut |p| {
        let m = fs::metadata(p).unwrap();
        out.push((p.strip_prefix(root).unwrap().to_string_lossy().into_owned(), fs::read(p).unwrap(), m.modified().unwrap()));
    });
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 폴더 아래 파일의 상대 경로 목록(정렬)
pub fn file_list(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if root.exists() {
        walk(root, &mut |p| v.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")));
    }
    v.sort();
    v
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() { walk(&p, f) } else { f(&p) }
    }
}

/// PID 로만 확인 — 이미지 이름으로 찾지 않는다
pub fn pid_alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output().expect("tasklist");
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

/// PID 로 종료 (이름으로 종료하지 않는다)
pub fn kill_pid(pid: u32) {
    let st = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(std::process::Stdio::null()).status().expect("taskkill");
    assert!(st.success());
}
