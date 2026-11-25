//! P5 시험용 `ProviderInvoker` — 실제 `PluginHost`(실제 워커) + 실제 `Gateway`. 활성·설치·자동 비활성을 시험이 바꾼다.
//! 호출 지점 훅(`before`/`after`)과 오류 주입으로 "실행 중 일시정지·종료·제거·편집" 같은 경합을 결정적으로 만든다.
//! 워커의 `mam.host.call('test.step' | 'test.kill')` 은 이 파일의 `TestApi` 가 처리하고 나머지는 게이트웨이로 보낸다.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mam_plugin_gateway::Gateway;
use mam_plugin_host::{CallCx, HostApi, HostError, HostState, InvokeError, InvokeKind, InvokeScope, LogLevel, PluginHost};
use mam_plugin_runtime::{Permission, ProviderInvoker, ProviderKind, ProviderSpec, RunError};
use serde_json::{Value, json};

/// 호스트 함수 `test.*` + 게이트웨이 위임
pub struct TestApi {
    gw: Arc<Gateway>,
    steps: Mutex<VecDeque<Value>>,
    kill_pid: AtomicU32,
}

impl TestApi {
    pub fn new(gw: Arc<Gateway>) -> Self {
        Self { gw, steps: Mutex::new(VecDeque::new()), kill_pid: AtomicU32::new(0) }
    }
}

impl HostApi for TestApi {
    fn call(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, HostError> {
        match method {
            "test.step" => Ok(self.steps.lock().unwrap().pop_front().unwrap_or_else(|| json!({}))),
            "test.kill" => {
                // 시험이 미리 알아 둔 워커 PID 만 PID 로 종료한다 (이미지 이름으로 종료하지 않는다)
                let pid = self.kill_pid.load(Ordering::SeqCst);
                assert!(pid != 0, "arm_kill must run first");
                let st = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(std::process::Stdio::null()).status().expect("taskkill");
                assert!(st.success());
                Ok(Value::Null)
            }
            _ => self.gw.call(cx, method, params),
        }
    }

    fn log(&self, cx: &CallCx<'_>, level: LogLevel, message: &str) {
        self.gw.log(cx, level, message);
    }
}

/// 기록된 provider 호출 1건
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// 1 부터
    pub n: usize,
    pub plugin: String,
    pub provider: String,
    pub asset: i64,
    pub revision: i64,
}

pub type Hook = Arc<dyn Fn(&Call) + Send + Sync>;

struct Entry {
    host: Arc<PluginHost>,
    specs: Vec<ProviderSpec>,
    enabled: bool,
    installed: bool,
}

pub struct TestInvoker {
    gw: Arc<Gateway>,
    pub api: Arc<TestApi>,
    entries: Mutex<BTreeMap<String, Entry>>,
    calls: Mutex<Vec<Call>>,
    injected: Mutex<VecDeque<InvokeError>>,
    before: Mutex<Option<Hook>>,
    after: Mutex<Option<Hook>>,
    export_in_auto: AtomicBool,
    seq: AtomicUsize,
}

impl TestInvoker {
    pub fn new(gw: Arc<Gateway>, api: Arc<TestApi>) -> Self {
        Self {
            gw,
            api,
            entries: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
            injected: Mutex::new(VecDeque::new()),
            before: Mutex::new(None),
            after: Mutex::new(None),
            export_in_auto: AtomicBool::new(false),
            seq: AtomicUsize::new(0),
        }
    }

    pub fn add(&self, id: &str, host: Arc<PluginHost>, specs: Vec<ProviderSpec>) {
        self.entries.lock().unwrap().insert(id.to_owned(), Entry { host, specs, enabled: true, installed: true });
    }

    pub fn host(&self, id: &str) -> Arc<PluginHost> {
        Arc::clone(&self.entries.lock().unwrap().get(id).expect("plugin").host)
    }

    pub fn set_enabled(&self, id: &str, on: bool) {
        self.entries.lock().unwrap().get_mut(id).expect("plugin").enabled = on;
    }

    pub fn set_installed(&self, id: &str, on: bool) {
        self.entries.lock().unwrap().get_mut(id).expect("plugin").installed = on;
    }

    /// 제품(P6)은 내보내기 provider 를 `auto_providers` 에 넣지 않는다 — 넣는 잘못된 호출자에게도 피드가 거르는지 시험한다
    pub fn leak_export_into_auto(&self, on: bool) {
        self.export_in_auto.store(on, Ordering::SeqCst);
    }

    /// 다음 호출들을 실제 워커 대신 이 오류로 끝낸다 (호출 수에는 센다)
    pub fn inject(&self, errors: Vec<InvokeError>) {
        self.injected.lock().unwrap().extend(errors);
    }

    pub fn set_before(&self, f: impl Fn(&Call) + Send + Sync + 'static) {
        *self.before.lock().unwrap() = Some(Arc::new(f));
    }

    pub fn set_after(&self, f: impl Fn(&Call) + Send + Sync + 'static) {
        *self.after.lock().unwrap() = Some(Arc::new(f));
    }

    pub fn clear_hooks(&self) {
        *self.before.lock().unwrap() = None;
        *self.after.lock().unwrap() = None;
    }

    /// `test.step` 가 돌려줄 값들 (호출 순서대로 하나씩 소비, 비면 `{}`)
    pub fn script(&self, steps: Vec<Value>) {
        self.api.steps.lock().unwrap().extend(steps);
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// 워커를 미리 띄워(`ping`) PID 를 알아 두고, 다음 `test.kill` 이 그 PID 를 종료하게 한다. PID 를 돌려준다
    pub fn arm_kill(&self, plugin: &str) -> u32 {
        let host = self.host(plugin);
        host.invoke(InvokeKind::Provider, "ping", Value::Null, InvokeScope::empty()).expect("warm-up ping");
        match host.state() {
            HostState::Running { pid } => {
                self.api.kill_pid.store(pid, Ordering::SeqCst);
                pid
            }
            other => panic!("worker should be running after ping: {other:?}"),
        }
    }

    pub fn shutdown_all(&self) {
        for e in self.entries.lock().unwrap().values() {
            e.host.shutdown();
        }
    }

    fn hook(slot: &Mutex<Option<Hook>>) -> Option<Hook> {
        slot.lock().unwrap().clone()
    }
}

impl ProviderInvoker for TestInvoker {
    fn auto_providers(&self) -> Vec<ProviderSpec> {
        let leak = self.export_in_auto.load(Ordering::SeqCst);
        let g = self.entries.lock().unwrap();
        g.values()
            .filter(|e| e.installed && e.enabled)
            .flat_map(|e| e.specs.iter().filter(move |s| leak || s.kind != ProviderKind::Export).cloned())
            .collect()
    }

    fn provider(&self, plugin: &str, provider: &str) -> Result<ProviderSpec, RunError> {
        let g = self.entries.lock().unwrap();
        let e = g.get(plugin).filter(|e| e.installed).ok_or(RunError::NotInstalled)?;
        if !e.enabled {
            return Err(RunError::Disabled);
        }
        e.specs.iter().find(|s| s.provider == provider).cloned().ok_or(RunError::UnknownTarget)
    }

    fn allows(&self, plugin: &str, perm: Permission) -> bool {
        self.gw.grants().allows(plugin, perm)
    }

    fn invoke_provider(&self, plugin: &str, provider: &str, assets: Vec<i64>, params: Value) -> Result<Value, RunError> {
        let call = Call {
            n: self.seq.fetch_add(1, Ordering::SeqCst) + 1,
            plugin: plugin.to_owned(),
            provider: provider.to_owned(),
            asset: params["assetId"].as_i64().unwrap_or(0),
            revision: params["revision"].as_i64().unwrap_or(0),
        };
        self.calls.lock().unwrap().push(call.clone());
        if let Some(h) = Self::hook(&self.before) {
            h(&call);
        }
        let host = {
            let g = self.entries.lock().unwrap();
            let e = g.get(plugin).filter(|e| e.installed).ok_or(RunError::NotInstalled)?;
            if !e.enabled {
                return Err(RunError::Disabled);
            }
            Arc::clone(&e.host)
        };
        let scope = InvokeScope::new(assets)?;
        let injected = self.injected.lock().unwrap().pop_front();
        let result = match injected {
            Some(e) => Err(e),
            None => host.invoke(InvokeKind::Provider, provider, params, scope),
        };
        if let Some(h) = Self::hook(&self.after) {
            h(&call);
        }
        match result {
            Ok(v) => Ok(v),
            Err(InvokeError::Disabled) => {
                self.set_enabled(plugin, false); // 자동 비활성: P6 와 같이 비활성으로 보고 `Disabled`
                Err(RunError::Disabled)
            }
            Err(e) => Err(RunError::Invoke(e)),
        }
    }
}
