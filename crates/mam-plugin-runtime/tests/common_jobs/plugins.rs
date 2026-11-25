//! 플러그인 정의·설치·활성/권한 변경·제거 경합 재현 — 시험이 `Env` 위에서 쓰는 관리 동작.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use mam_db::Priority;
use mam_db::plugin_settings::{self, ENABLED, GRANTED, INSTALLED};
use mam_plugin_host::{BreakerConfig, HostConfig, SystemClock};
use mam_plugin_manifest::parse_manifest;
use mam_plugin_runtime::{AssetKind, JobHooks, Permission, PluginId, ProviderKind, ProviderSpec};

use super::Env;

/// 워커 바이너리 — 시험 프로세스 안에서 한 번 `cargo build -p mam-plugin-worker` (최신이면 즉시 끝난다)
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

pub fn js(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("jobs").join(format!("{name}.js"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {}: {e}", path.display())).replace("\r\n", "\n")
}

pub fn plugin_id(id: &str) -> PluginId {
    let json = format!(r#"{{"apiVersion":1,"id":"{id}","version":"1.0.0","engines":{{"mam":">=0.1.0"}},"main":"main.js","displayName":"plugin.name"}}"#);
    parse_manifest(&json).expect("valid manifest").id
}

/// 설치할 플러그인 정의 (필드를 직접 바꿔 쓴다)
pub struct Plugin {
    pub id: String,
    pub source: String,
    pub providers: Vec<(String, ProviderKind, Vec<AssetKind>)>,
    /// `None` = 선언한 권한 전부
    pub granted: Option<Vec<Permission>>,
    pub call_timeout: Option<Duration>,
    pub cpu_ms: Option<u64>,
    pub breaker: Option<BreakerConfig>,
    /// 제품처럼 설치 훅(`installed(fresh=true)`)을 부른다
    pub install_hook: bool,
}

impl Plugin {
    pub fn new(id: &str, fixture: &str) -> Self {
        Self {
            id: id.to_owned(),
            source: js(fixture),
            providers: Vec::new(),
            granted: None,
            call_timeout: None,
            cpu_ms: None,
            breaker: None,
            install_hook: true,
        }
    }

    pub fn provider(mut self, id: &str, kind: ProviderKind, kinds: &[AssetKind]) -> Self {
        self.providers.push((id.to_owned(), kind, kinds.to_vec()));
        self
    }
}

impl Env {
    // ── 플러그인 ─────────────────────────────────────────────

    /// 플러그인을 설치한다 — 설정 행(installed·enabled·granted)·권한 등록·호스트·(선택) 설치 훅
    pub fn install(&self, p: Plugin) -> PluginId {
        let pid = plugin_id(&p.id);
        let manifest: BTreeSet<Permission> = Permission::ALL.into_iter().collect();
        self.gw.grants().register(&pid, &manifest);
        let granted = p.granted.clone().unwrap_or_else(|| Permission::ALL.to_vec());
        let id = p.id.clone();
        let raw = granted_json(&granted);
        self.pool
            .write(Priority::High, move |c| {
                plugin_settings::put(c, INSTALLED, &id, r#"{"version":"1.0.0","installedAt":1}"#).unwrap();
                plugin_settings::put(c, ENABLED, &id, "true").unwrap();
                plugin_settings::put(c, GRANTED, &id, &raw).unwrap();
                Ok(())
            })
            .unwrap();
        self.gw.grants().refresh(&self.pool, &pid, Instant::now() + Duration::from_secs(10)).unwrap();
        let mut cfg = HostConfig::new(worker_exe(), p.source.clone());
        if let Some(t) = p.call_timeout {
            cfg.call_timeout = t;
        }
        if let Some(c) = p.cpu_ms {
            cfg.limits.cpu_ms = c;
        }
        if let Some(b) = p.breaker {
            cfg.breaker = b;
        }
        let host = Arc::new(mam_plugin_host::PluginHost::new(pid.clone(), cfg, Arc::clone(&self.invoker.api) as _, Arc::new(SystemClock)));
        let specs = p
            .providers
            .iter()
            .map(|(id, kind, kinds)| ProviderSpec {
                plugin: pid.clone(),
                provider: id.clone(),
                kind: *kind,
                asset_kinds: kinds.clone(),
                file_extension: (*kind == ProviderKind::Export).then(|| "csv".to_owned()),
            })
            .collect();
        self.invoker.add(&p.id, host, specs);
        if p.install_hook {
            self.jobs.installed(&pid, true);
        }
        pid
    }

    /// 사용자가 끈다 — 설정·권한 갱신·호출 불가
    pub fn disable(&self, id: &str) {
        self.set_enabled_setting(id, false);
        self.invoker.set_enabled(id, false);
    }

    /// 사용자가 켠다 — 설정·권한 갱신 뒤 `enabled` 훅 (대기 작업 `jobs` 재등록)
    pub fn enable(&self, id: &str) {
        self.set_enabled_setting(id, true);
        self.invoker.set_enabled(id, true);
        self.jobs.enabled(&plugin_id(id));
    }

    fn set_enabled_setting(&self, id: &str, on: bool) {
        let key = id.to_owned();
        self.pool
            .write(Priority::High, move |c| {
                plugin_settings::put(c, ENABLED, &key, if on { "true" } else { "false" }).unwrap();
                Ok(())
            })
            .unwrap();
        self.gw.grants().refresh(&self.pool, &plugin_id(id), Instant::now() + Duration::from_secs(10)).unwrap();
    }

    /// 허용 권한을 바꾼다 (게이트 ON: 플러그인은 활성 상태 그대로)
    pub fn set_granted(&self, id: &str, granted: &[Permission]) {
        let (key, raw) = (id.to_owned(), granted_json(granted));
        self.pool.write(Priority::High, move |c| {
                plugin_settings::put(c, GRANTED, &key, &raw).unwrap();
                Ok(())
            })
            .unwrap();
        self.gw.grants().refresh(&self.pool, &plugin_id(id), Instant::now() + Duration::from_secs(10)).unwrap();
    }

    /// 제거의 첫 단계 — 설정 행(installed·enabled·granted·cursor)을 한 트랜잭션으로 지운다 (런타임 항목은 그대로 — 경합 재현용)
    pub fn delete_plugin_settings(&self, id: &str) {
        let key = id.to_owned();
        self.pool.write(Priority::High, move |c| {
                plugin_settings::delete_plugin(c, &key).unwrap();
                Ok(())
            })
            .unwrap();
    }

    /// `delete_plugin_settings` 를 다른 스레드·훅에서 부르는 클로저 (제거 경합 재현용)
    pub fn settings_deleter(&self, id: &str) -> impl Fn() + Send + Sync + 'static {
        let (pool, id) = (Arc::clone(&self.pool), id.to_owned());
        move || {
            let id = id.clone();
            pool.write(Priority::High, move |c| {
                plugin_settings::delete_plugin(c, &id).unwrap();
                Ok(())
            })
            .unwrap()
        }
    }

}

pub fn granted_json(perms: &[Permission]) -> String {
    serde_json::json!(perms.iter().map(|p| p.as_str()).collect::<Vec<_>>()).to_string()
}

/// 시작할 때 아직 `PluginJobs` 설치 훅이 없는 상태로 설치한다 (지연 커서 초기화 시험)
pub fn without_hook(mut p: Plugin) -> Plugin {
    p.install_hook = false;
    p
}

