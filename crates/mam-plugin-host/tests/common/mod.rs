//! 통합 시험 공용 — 워커 바이너리 위치, 가짜 호스트 API, 가짜 시계, 호스트 생성 도우미.
//!
//! 워커 위치(결정): 이 crate 의 시험 실행 파일은 `<target>/<profile>/deps/` 에 있고 `mam-plugin-worker.exe` 는 `<target>/<profile>/` 에 놓인다.
//! `CARGO_BIN_EXE_*` 는 같은 패키지의 바이너리에만 정의되므로 쓸 수 없다 — 그래서 시험 프로세스 안에서 한 번 `cargo build -p mam-plugin-worker`
//! (현재 프로필과 같게)를 돌려 최신을 보장한다. 이미 최신이면 즉시 끝난다. `MAM_PLUGIN_WORKER_EXE` 로 경로를 직접 줄 수도 있다.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use mam_plugin_host::{CallCx, Clock, HostApi, HostConfig, HostError, LogLevel, PluginHost};
use mam_plugin_manifest::{PluginId, parse_manifest};
use serde_json::{Value, json};

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
        let status = cmd.status().expect("run cargo build for the worker");
        assert!(status.success(), "building mam-plugin-worker failed");
        let exe = profile_dir.join("mam-plugin-worker.exe");
        assert!(exe.exists(), "worker not found at {}", exe.display());
        exe
    })
    .clone()
}

pub fn plugin_id(id: &str) -> PluginId {
    let json = format!(r#"{{"apiVersion":1,"id":"{id}","version":"0.1.0","engines":{{"mam":">=0.1.0"}},"main":"main.js","displayName":"plugin.name"}}"#);
    parse_manifest(&json).expect("valid manifest").id
}

/// 가짜 호스트 API. 메서드: echo / fail / sleep(ms) / kill_me(미리 정한 PID 를 PID 로 종료)
#[derive(Default)]
pub struct FakeApi {
    pub kill_pid: Mutex<Option<u32>>,
    pub calls: Mutex<Vec<(String, String)>>,
    pub logs: Mutex<Vec<(LogLevel, String)>>,
    pub scopes: Mutex<Vec<(Vec<i64>, Instant)>>,
}

impl HostApi for FakeApi {
    fn call(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, HostError> {
        self.scopes.lock().unwrap().push((cx.scope.assets.to_vec(), cx.scope.deadline));
        self.calls.lock().unwrap().push((cx.plugin.as_str().to_owned(), method.to_owned()));
        match method {
            "echo" => Ok(params),
            "fail" => Err(HostError::new("denied", "nope")),
            "sleep" => {
                std::thread::sleep(Duration::from_millis(params.as_u64().unwrap_or(0)));
                Ok(Value::Null)
            }
            "kill_me" => {
                let pid = self.kill_pid.lock().unwrap().expect("pid set");
                let st = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(std::process::Stdio::null()).status().expect("taskkill");
                assert!(st.success());
                Ok(Value::Null)
            }
            _ => Err(HostError::new("not_found", "unknown host method")),
        }
    }

    fn log(&self, _cx: &CallCx<'_>, level: LogLevel, message: &str) {
        self.logs.lock().unwrap().push((level, message.to_owned()));
    }
}

/// 가짜 시계 — 기준 Instant + 수동으로 늘리는 오프셋
pub struct FakeClock {
    base: Instant,
    offset: Mutex<Duration>,
}

impl FakeClock {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { base: Instant::now(), offset: Mutex::new(Duration::ZERO) })
    }
    pub fn advance(&self, d: Duration) {
        *self.offset.lock().unwrap() += d;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.base + *self.offset.lock().unwrap()
    }
}

/// 공용 플러그인 소스 — 명령 모음
pub const PLUGIN: &str = r#"
mam.commands.register('echo', (p) => ({ echoed: p }));
mam.commands.register('hostcall', (p) => ({ got: mam.host.call('echo', { n: p.n }) }));
mam.commands.register('hosterr', () => { try { mam.host.call('fail', null); return 'no error'; } catch (e) { return { code: e.code, message: e.message }; } });
mam.commands.register('unknown_host', () => mam.host.call('does.not.exist', null));
mam.commands.register('throw', () => { throw new Error('boom'); });
mam.commands.register('async', () => Promise.resolve(1));
mam.commands.register('kill_me', () => mam.host.call('kill_me', null));
mam.commands.register('spin', () => { for (;;) {} });
mam.commands.register('bomb', () => { const a = []; for (;;) a.push('x'.repeat(1 << 20) + a.length); });
mam.commands.register('bigresult', () => 'x'.repeat(2 * 1024 * 1024));
mam.commands.register('flood', () => { for (let i = 0; i < 100000; i++) mam.log.info('spam'); return 1; });
mam.commands.register('logs', () => { mam.log.warn('w', { a: 1 }); mam.log.error('가'.repeat(3000)); return 1; });
mam.commands.register('busy_with_calls', (p) => {
  for (let i = 0; i < p.rounds; i++) {
    mam.host.call('sleep', p.sleepMs);
    const t = Date.now(); while (Date.now() - t < p.busyMs) {}
  }
  return 'done';
});
mam.providers.register('meta.extract', (p) => ({ kind: 'provider', p }));
"#;

pub fn new_host(source: &str, tweak: impl FnOnce(&mut HostConfig)) -> (PluginHost, Arc<FakeApi>, Arc<FakeClock>) {
    let api = Arc::new(FakeApi::default());
    let clock = FakeClock::new();
    let mut cfg = HostConfig::new(worker_exe(), source.to_owned());
    tweak(&mut cfg);
    let host = PluginHost::new(plugin_id("com.example.test"), cfg, api.clone(), clock.clone());
    (host, api, clock)
}

pub fn run(host: &PluginHost, cmd: &str, params: Value) -> Result<Value, mam_plugin_host::InvokeError> {
    host.invoke(mam_plugin_host::InvokeKind::Command, cmd, params, mam_plugin_host::InvokeScope::empty())
}

pub fn ok_echo(host: &PluginHost) {
    assert_eq!(run(host, "echo", json!(1)).unwrap(), json!({"echoed": 1}));
}
