//! 플러그인 1개의 워커 감독자 — 지연 시작, 호출 타임아웃(워커 종료 후 다음 호출에서 재시작), 크래시 감지, 10분 3회 비활성.
//! 호출은 플러그인당 직렬이다(락). 호출자는 UI 스레드가 아닌 작업 스레드(`spawn_blocking`)여야 한다.
//! 마감·차단기 시각: 파이프 대기 마감은 실제 시계, 차단기 판정은 주입된 `Clock`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use mam_plugin_manifest::PluginId;
use serde_json::Value;

use crate::activity::{ActivityLog, HostActivity};
use crate::breaker::{Breaker, BreakerConfig, Clock};
use crate::call_loop::{CallCtx, run_call};
use crate::error::InvokeError;
use crate::host_api::{HostApi, InvokeScope};
use crate::line_reader::LineEvent;
use crate::protocol::{HostMsg, InvokeKind, Limits, PROTOCOL_VERSION, WorkerMsg, parse_worker_line};
use crate::worker_proc::WorkerProc;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub worker_exe: PathBuf,
    /// 플러그인 main JS 소스 본문 — 읽기·경로 검증은 로더(P6) 몫. 워커는 파일을 열지 않는다
    pub source: String,
    pub limits: Limits,
    pub start_timeout: Duration,
    pub call_timeout: Duration,
    /// 호출 1회에서 워커가 보낼 수 있는 메시지(host_call·log 포함) 수 — 넘으면 폭주로 보고 종료
    pub max_messages_per_call: usize,
    pub breaker: BreakerConfig,
}

impl HostConfig {
    /// 제안 기본값(미측정): 힙 64 MiB, 스택 512 KiB, CPU 2 s/호출, 줄 4 MiB, 시작 10 s, 호출 10 s
    pub fn new(worker_exe: PathBuf, source: String) -> Self {
        Self {
            worker_exe,
            source,
            limits: Limits { memory_bytes: 64 << 20, stack_bytes: 512 << 10, cpu_ms: 2000, max_line_bytes: 4 << 20 },
            start_timeout: Duration::from_secs(10),
            call_timeout: Duration::from_secs(10),
            max_messages_per_call: 10_000,
            breaker: BreakerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostState {
    /// 워커 없음(다음 호출에서 시작)
    Idle,
    Running { pid: u32 },
    Disabled,
}

struct State {
    live: Option<WorkerProc>,
    next_id: u64,
    breaker: Breaker,
    starts: u32,
}

pub struct PluginHost {
    id: PluginId,
    cfg: HostConfig,
    api: Arc<dyn HostApi>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    activity: ActivityLog,
}

impl PluginHost {
    pub fn new(id: PluginId, cfg: HostConfig, api: Arc<dyn HostApi>, clock: Arc<dyn Clock>) -> Self {
        let breaker = Breaker::new(cfg.breaker);
        Self { id, cfg, api, clock, state: Mutex::new(State { live: None, next_id: 1, breaker, starts: 0 }), activity: ActivityLog::default() }
    }

    /// 시험 전용 — 호출 진입·종료 요청 횟수
    #[doc(hidden)]
    pub fn activity(&self) -> HostActivity {
        self.activity.snapshot()
    }

    /// 시험 전용 — 조건 변수로 활동 조건을 기다린다(폴링 없음). 제한 시간 안에 안 되면 false
    #[doc(hidden)]
    pub fn wait_activity(&self, timeout: Duration, pred: impl Fn(&HostActivity) -> bool) -> bool {
        self.activity.wait_until(timeout, pred)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn state(&self) -> HostState {
        let mut s = self.lock();
        if s.breaker.is_disabled() {
            return HostState::Disabled;
        }
        match s.live.as_mut() {
            Some(w) => {
                if w.has_exited() { HostState::Idle } else { HostState::Running { pid: w.pid() } }
            }
            None => HostState::Idle,
        }
    }

    /// 워커를 시작한 횟수(재시작 포함)
    pub fn starts(&self) -> u32 {
        self.lock().starts
    }

    /// 사용자가 다시 켰을 때 — 차단기 이력을 지운다
    pub fn reenable(&self) {
        self.lock().breaker.reset();
    }

    /// 워커를 종료한다(앱 종료·비활성화). 다음 호출이 있으면 다시 시작된다
    pub fn shutdown(&self) {
        self.activity.bump(|a| a.shutdowns_requested += 1);
        if let Some(mut w) = self.lock().live.take() {
            w.kill();
        }
    }

    pub fn invoke(&self, kind: InvokeKind, target: &str, params: Value, scope: InvokeScope) -> Result<Value, InvokeError> {
        self.invoke_with_timeout(kind, target, params, scope, self.cfg.call_timeout)
    }

    pub fn invoke_with_timeout(&self, kind: InvokeKind, target: &str, params: Value, scope: InvokeScope, timeout: Duration) -> Result<Value, InvokeError> {
        let mut st = self.lock();
        self.activity.bump(|a| a.invokes_started += 1);
        if st.breaker.is_disabled() {
            return Err(InvokeError::Disabled);
        }
        let result = self.invoke_locked(&mut st, kind, target, params, scope, timeout);
        if let Err(e) = &result
            && e.is_fault()
        {
            if let Some(mut w) = st.live.take() {
                w.kill();
            }
            let now = self.clock.now();
            st.breaker.record(now);
        }
        result
    }

    fn invoke_locked(&self, st: &mut State, kind: InvokeKind, target: &str, params: Value, mut scope: InvokeScope, timeout: Duration) -> Result<Value, InvokeError> {
        // 유휴 중 워커가 죽었으면(외부 종료·크래시) 결함으로 기록하고 새로 시작한다
        if st.live.as_mut().is_some_and(|w| w.has_exited()) {
            st.live = None;
            let now = self.clock.now();
            if st.breaker.record(now) {
                return Err(InvokeError::Disabled);
            }
        }
        if st.live.is_none() {
            let w = self.start_worker()?;
            st.live = Some(w);
            st.starts += 1;
        }
        let id = st.next_id;
        st.next_id += 1;
        let Some(w) = st.live.as_mut() else { return Err(InvokeError::Crashed) };
        let known = match kind {
            InvokeKind::Command => &w.handlers.commands,
            InvokeKind::Provider => &w.handlers.providers,
        };
        if !known.contains(target) {
            return Err(InvokeError::NotRegistered(target.to_owned()));
        }
        w.send(&HostMsg::Invoke { id, kind, target: target.to_owned(), params }).map_err(|_| InvokeError::Crashed)?;
        // 호출자가 만든 범위의 마감을 호출 타임아웃으로 찍는다 — 호스트 함수의 DB 대기가 이 시각을 넘지 않는다
        scope.deadline = Instant::now() + timeout;
        let ctx = CallCtx { plugin: &self.id, api: self.api.as_ref(), scope: &scope, max_messages: self.cfg.max_messages_per_call };
        run_call(w, &ctx, id)
    }

    /// 워커를 띄우고 `init` -> `ready` 핸드셰이크. 실패 시 워커는 정리하고(Drop) 오류를 돌려준다
    fn start_worker(&self) -> Result<WorkerProc, InvokeError> {
        let max_line = usize::try_from(self.cfg.limits.max_line_bytes).unwrap_or(usize::MAX);
        let mut w = WorkerProc::spawn(&self.cfg.worker_exe, max_line, self.cfg.limits.memory_bytes).map_err(|e| InvokeError::Start(e.kind().to_string()))?;
        let init = HostMsg::Init { v: PROTOCOL_VERSION, plugin_id: self.id.as_str().to_owned(), source: self.cfg.source.clone(), limits: self.cfg.limits };
        w.send(&init).map_err(|_| InvokeError::Crashed)?;
        let ev = w.rx.recv_timeout(self.cfg.start_timeout).map_err(|_| InvokeError::Timeout)?;
        let line = match ev {
            LineEvent::Line(l) => l,
            LineEvent::Oversize => return Err(InvokeError::Oversize),
            LineEvent::Eof => return Err(InvokeError::Crashed),
        };
        match parse_worker_line(&line).map_err(InvokeError::Protocol)? {
            WorkerMsg::Ready { v, commands, providers } if v == PROTOCOL_VERSION => {
                w.handlers.commands = commands.into_iter().collect();
                w.handlers.providers = providers.into_iter().collect();
                Ok(w)
            }
            WorkerMsg::Ready { v, .. } => Err(InvokeError::Protocol(format!("worker protocol version {v}"))),
            WorkerMsg::InitError { message } => Err(InvokeError::Init(message)),
            _ => Err(InvokeError::Protocol("expected ready".into())),
        }
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}
