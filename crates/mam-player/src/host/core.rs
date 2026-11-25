//! 감독자 스레드의 상태 기계 — 모든 상태는 이 스레드 한 곳에서만 바뀐다 (잠금 없음).
//! 입력은 `Inbox` 하나: 호출자 요청 · 워커 메시지 · 프레임 도착 알림. 기다림은 전부 상태(`Phase`)와 마감 시각으로 표현하고
//! 감독자 스레드는 spawn·파이프 연결·`create_bank` 대기를 빼면 막히지 않는다 (사용자 요청은 큐에서 곧바로 처리된다).
//! 이 파일은 상태·메시지 처리, `ops.rs` 는 사용자 요청, `lifecycle.rs` 는 열기·복원·재시작을 맡는다.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use super::events::{Emit, EventBridge, EventState, PlayerErrorCode, PlayerEvent};
use super::link::{Inbox, Inc, Shared};
use super::proc::WorkerSpawner;
use super::restore::{Remembered, RestorePlan};
use super::surface::FrameSurface;
use super::types::{HostConfig, OpenInfo, OpenRequest};
use crate::PlayerError;
use crate::protocol::{Command, ControlAction, Message, RangeMs, WorkerErrorCode};

pub(crate) type ReplyTx = Sender<Result<Reply, PlayerError>>;

#[derive(Debug)]
pub(crate) enum Reply {
    Unit,
    Open(OpenInfo),
}

pub(crate) struct Req {
    pub kind: ReqKind,
    pub reply: Option<ReplyTx>,
}

pub(crate) enum ReqKind {
    Open(OpenRequest),
    SetRanges { asset_id: i64, ranges: Option<Vec<RangeMs>> },
    SetSubtitle { asset_id: i64, path: Option<std::path::PathBuf> },
    Control(ControlAction),
    Seek(i64),
    SetRate(f64),
    SetVolume { volume: u8, muted: bool },
    SetRenderSize(u32, u32),
    SetRenderActive(bool),
    Restart,
    Close,
    Shutdown,
    /// 시험 전용 — 워커를 비정상 종료시킨다
    #[cfg(any(test, feature = "test-support"))]
    KillWorker,
}

/// 응답을 기다리는 명령이 끝났을 때 할 일
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Done {
    /// 호출자에게 알린다
    Reply,
    /// `Attach` 응답 — 성공하면 `old` epoch 의 버퍼를 닫는다
    Attach { new: u64, old: u64, purpose: AttachFor },
    /// `Open` 접수 응답 (실패면 열기 실패)
    OpenAck,
    CloseAck,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachFor {
    Open,
    Restore,
    Resize,
}

pub(crate) struct Pending {
    pub id: u64,
    pub cmd: Command,
    pub done: Done,
    pub reply: Option<ReplyTx>,
    pub deadline: Instant,
    /// 한 번 더 보낼 수 있는가 (멱등 명령만)
    pub retry: bool,
    pub timeout: Duration,
}

pub(crate) struct OpenWait {
    pub reply: ReplyTx,
    pub deadline: Instant,
    pub close_gen: u64,
    pub attached: bool,
    pub loaded: bool,
    pub restart: Option<(i64, Instant)>,
    pub baseline: u64,
}

pub(crate) struct RestoreWait {
    pub plan: RestorePlan,
    pub deadline: Instant,
    pub attached: bool,
    pub baseline: u64,
    pub restart_at: Option<Instant>,
}

pub(crate) enum Phase {
    /// 세션 없음 (워커는 떠 있을 수 있다)
    Idle,
    Opening(Box<OpenWait>),
    Active,
    Restoring(Box<RestoreWait>),
    /// 재시작 전 백오프
    Backoff { until: Instant },
    /// 연속 실패로 포기 — 열기/재시작/닫기만 받는다
    Failed,
}

pub(crate) struct Core {
    pub cfg: HostConfig,
    pub surface: Arc<dyn FrameSurface>,
    pub spawner: Option<Arc<dyn WorkerSpawner>>,
    pub shared: Arc<Shared>,
    pub tx: Sender<Inbox>,
    pub rx: Receiver<Inbox>,
    pub bridge: EventBridge,
    pub inc: Option<Inc>,
    pub next_inc: u64,
    pub next_epoch: u64,
    pub cur_epoch: u64,
    pub session_counter: u64,
    pub rem: Remembered,
    pub phase: Phase,
    pub pending: Vec<Pending>,
    /// 연속 재시작 시도 횟수 (복원 + 프레임 도착으로만 0 이 된다)
    pub streak: u8,
    /// 수동 `player_restart` 의 응답 (자동 재시작 시도를 거쳐 끝날 때 답한다)
    pub restart_reply: Option<ReplyTx>,
    pub ended: bool,
    pub last_epoch_change: Option<Instant>,
    pub queued_size: Option<(u32, u32)>,
    /// 시험·진단용 — 마지막 워커 종료 사유
    pub last_down: Option<&'static str>,
}

impl Core {
    pub fn new(
        cfg: HostConfig,
        surface: Arc<dyn FrameSurface>,
        spawner: Option<Arc<dyn WorkerSpawner>>,
        shared: Arc<Shared>,
        tx: Sender<Inbox>,
        rx: Receiver<Inbox>,
        sink: super::events::EventSink,
    ) -> Self {
        Self {
            cfg,
            surface,
            spawner,
            shared,
            tx,
            rx,
            bridge: EventBridge::new(sink),
            inc: None,
            next_inc: 0,
            next_epoch: 1,
            cur_epoch: 0,
            session_counter: 0,
            rem: Remembered::default(),
            phase: Phase::Idle,
            pending: Vec::new(),
            streak: 0,
            restart_reply: None,
            ended: false,
            last_epoch_change: None,
            queued_size: None,
            last_down: None,
        }
    }

    pub fn run(mut self) {
        loop {
            let wait = self.next_wait(Instant::now());
            match self.rx.recv_timeout(wait) {
                Ok(Inbox::Req(r)) => {
                    if self.on_req(r) {
                        break;
                    }
                }
                Ok(Inbox::Worker { inc, msg }) => self.on_worker(inc, msg),
                Ok(Inbox::FrameSeen) => self.after_progress(),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.tick(Instant::now());
        }
        self.teardown_worker(true);
    }

    /// 다음 깨어날 시각까지의 대기 — 마감·백오프·trailing 이벤트 중 가장 가까운 것 (최대 100ms)
    fn next_wait(&self, now: Instant) -> Duration {
        let mut due = now + Duration::from_millis(100);
        let mut consider = |t: Instant| {
            if t < due {
                due = t;
            }
        };
        for p in &self.pending {
            consider(p.deadline);
        }
        if let Some(t) = self.bridge.next_due() {
            consider(t);
        }
        if let Phase::Backoff { until } = &self.phase {
            consider(*until);
        }
        due.saturating_duration_since(now).max(Duration::from_millis(1))
    }

    // ---------- 이벤트 ----------

    pub fn state_event(&self) -> PlayerEvent {
        let state = if self.ended {
            EventState::Ended
        } else if self.rem.paused {
            EventState::Paused
        } else {
            EventState::Playing
        };
        let mut e = PlayerEvent::new(self.rem.session_id, self.rem.asset_id, state, self.rem.pos_ms, self.rem.length_ms, self.rem.fps);
        e.buffering = self.rem.buffering;
        e
    }

    pub fn emit_state(&mut self, kind: Emit) {
        if self.rem.has_session() {
            let ev = self.state_event();
            self.bridge.emit(ev, kind, Instant::now());
        }
    }

    // ---------- 워커 메시지 ----------

    fn on_worker(&mut self, inc: u64, msg: Option<Message>) {
        if self.inc.as_ref().map(|i| i.id) != Some(inc) {
            return; // 이미 정리한 세대의 늦은 메시지
        }
        let Some(msg) = msg else {
            self.worker_failed("pipe closed");
            return;
        };
        match &msg {
            Message::Reply { id, ok, code, error, .. } => {
                self.on_reply(*id, *ok, *code, error.clone());
                return;
            }
            Message::Error { code, msg } => {
                self.on_worker_error(*code, msg.clone());
                return;
            }
            _ => {}
        }
        match &mut self.phase {
            Phase::Opening(ow) => {
                match &msg {
                    Message::FileLoaded { .. } => ow.loaded = true,
                    Message::PlaybackRestart { time_ms } if ow.loaded => ow.restart = Some((*time_ms, Instant::now())),
                    _ => {}
                }
                self.rem.on_message(&msg);
            }
            Phase::Restoring(rw) => {
                if let Message::PlaybackRestart { .. } = &msg {
                    rw.restart_at = Some(Instant::now());
                }
                if let Message::FileLoaded { fps, duration_ms, .. } = &msg {
                    self.rem.fps = *fps;
                    self.rem.length_ms = *duration_ms;
                }
                let cmds = rw.plan.on_message(&msg);
                if let Some(i) = self.inc.as_mut() {
                    for c in cmds {
                        let _ = i.send(c, false);
                    }
                }
            }
            Phase::Active => self.on_active_message(&msg),
            Phase::Idle | Phase::Backoff { .. } | Phase::Failed => {}
        }
        self.after_progress();
    }

    fn on_active_message(&mut self, msg: &Message) {
        self.rem.on_message(msg);
        match msg {
            Message::PlaybackRestart { .. } => {
                self.ended = false;
                self.emit_state(Emit::Completion);
            }
            Message::Paused { paused, .. } => {
                if !paused {
                    self.ended = false;
                }
                self.emit_state(Emit::Transition);
            }
            Message::Ended { .. } => {
                self.ended = true;
                self.emit_state(Emit::Transition);
            }
            Message::Buffering { .. } => self.emit_state(Emit::Transition),
            Message::Time { .. } => {
                if !self.rem.paused && !self.ended {
                    self.emit_state(Emit::Time);
                }
            }
            _ => {}
        }
    }

    fn on_worker_error(&mut self, code: WorkerErrorCode, msg: String) {
        match &self.phase {
            Phase::Opening(_) => self.fail_open(PlayerError::OpenFailed(msg)),
            Phase::Active => {
                // 재생 중 읽기 오류 — 오류 이벤트 (세션은 유지, 사용자가 닫는다)
                let mut e = self.state_event();
                e.state = EventState::Error;
                e.error = Some("cannot read the media".into());
                e.error_code = Some(PlayerErrorCode::OpenFailed);
                self.bridge.emit(e, Emit::Transition, Instant::now());
                let _ = code;
            }
            _ => {}
        }
    }

    fn on_reply(&mut self, id: u64, ok: bool, code: Option<WorkerErrorCode>, error: Option<String>) {
        let Some(pos) = self.pending.iter().position(|p| p.id == id) else { return };
        let p = self.pending.remove(pos);
        let err = || {
            let m = error.clone().unwrap_or_else(|| "worker rejected the command".into());
            match code {
                Some(WorkerErrorCode::InvalidArgument) => PlayerError::InvalidArgument(m),
                Some(WorkerErrorCode::NoMedia) => PlayerError::NoMedia,
                Some(WorkerErrorCode::OpenFailed) => PlayerError::OpenFailed(m),
                _ => PlayerError::CommandFailed(m),
            }
        };
        match p.done {
            Done::Reply | Done::CloseAck => {
                if let Some(r) = p.reply {
                    let _ = r.send(if ok || p.done == Done::CloseAck { Ok(Reply::Unit) } else { Err(err()) });
                }
            }
            Done::Ignore => {}
            Done::OpenAck => {
                if !ok {
                    self.fail_open(err());
                }
            }
            Done::Attach { new, old, purpose } => {
                if ok {
                    self.on_attached(new, old, purpose);
                } else {
                    self.on_attach_failed(new, old, purpose, err());
                }
            }
        }
    }

    /// 진행 상황이 바뀐 뒤 열기·복원 완료 조건을 확인한다
    pub fn after_progress(&mut self) {
        self.try_finish_open(Instant::now());
        self.try_finish_restore(Instant::now());
    }

    // ---------- 시간 ----------

    pub fn tick(&mut self, now: Instant) {
        self.bridge.poll(now);
        self.expire_pending(now);
        // 열기 취소·마감·프레임 유예
        let mut open_failure = None;
        if let Phase::Opening(ow) = &self.phase {
            if self.shared.close_gen.load(std::sync::atomic::Ordering::Acquire) != ow.close_gen {
                open_failure = Some((PlayerError::Cancelled, false));
            } else if now >= ow.deadline {
                open_failure = Some((PlayerError::Timeout, true));
            }
        }
        if let Some((e, kill)) = open_failure {
            self.fail_open(e);
            if kill {
                self.worker_failed("open timed out");
            } else if let Some(i) = self.inc.as_mut() {
                let _ = i.send(Command::Close, false);
            }
        }
        self.try_finish_open(now);
        let restore_expired = matches!(&self.phase, Phase::Restoring(rw) if now >= rw.deadline);
        if restore_expired {
            self.worker_failed("restore timed out");
        }
        self.try_finish_restore(now);
        if let Phase::Backoff { until } = &self.phase
            && now >= *until
        {
            self.begin_restore();
        }
        if self.inc.is_some() && !matches!(self.phase, Phase::Idle | Phase::Failed) && self.shared.silence() > self.cfg.timeouts.silence {
            self.worker_failed("silence");
        }
        self.apply_queued_resize(now);
    }

    fn expire_pending(&mut self, now: Instant) {
        let due: Vec<usize> = self.pending.iter().enumerate().filter(|(_, p)| now >= p.deadline).map(|(i, _)| i).collect();
        let mut kill = false;
        for i in due.into_iter().rev() {
            let mut p = self.pending.remove(i);
            if p.retry {
                // 멱등 명령은 새 id 로 한 번 더 보낸다
                if let Some(id) = self.inc.as_mut().and_then(|inc| inc.send(p.cmd.clone(), true)) {
                    p.id = id;
                    p.retry = false;
                    p.deadline = now + p.timeout;
                    self.pending.push(p);
                    continue;
                }
            }
            if let Some(r) = p.reply.take() {
                // 닫기는 응답이 없어도 파일을 놓은 것으로 본다 (워커를 죽이므로) — 항상 Ok
                let _ = r.send(if p.done == Done::CloseAck { Ok(Reply::Unit) } else { Err(PlayerError::Timeout) });
            }
            match p.done {
                Done::Attach { new, old, purpose } => self.on_attach_failed(new, old, purpose, PlayerError::Timeout),
                Done::OpenAck => self.fail_open(PlayerError::Timeout),
                _ => {}
            }
            kill = true;
        }
        if kill {
            self.worker_failed("command timed out");
        }
    }
}
