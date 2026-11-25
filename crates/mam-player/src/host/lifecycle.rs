//! 열기 · 워커 시작 · 버퍼(epoch) 교체 · 자동 재시작과 복원 (계약 §3.6, §7, §10).
//! 모두 감독자 스레드에서 실행되고, 기다림은 `Phase` 와 마감 시각으로 표현한다.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::core::{AttachFor, Core, Done, OpenWait, Pending, Phase, Reply, ReplyTx, RestoreWait};
use super::events::{Emit, EventState, PlayerErrorCode, RestartInfo};
use super::link;
use super::restore::Remembered;
use super::surface::{BankAttach, SurfaceError};
use super::types::{MAX_RESTARTS, OpenInfo, OpenRequest};
use crate::PlayerError;
use crate::protocol::{Command, Message, PROTOCOL_VERSION, RENDER_CAP, fit_render_size};

fn surface_error(e: SurfaceError) -> PlayerError {
    match e {
        SurfaceError::Timeout => PlayerError::Timeout,
        SurfaceError::Unsupported => {
            PlayerError::SurfaceUnavailable("shared buffers are not supported".into())
        }
        SurfaceError::Failed(m) => PlayerError::SurfaceUnavailable(m),
    }
}

impl Core {
    // ---------- 워커 ----------

    /// 워커가 없으면 띄우고 `Ready` 를 기다린다 (감독자 스레드에서 막힌다 — UI 스레드가 아니다)
    pub fn ensure_worker(&mut self) -> Result<(), PlayerError> {
        if let Some(inc) = self.inc.as_mut() {
            if !inc.process.has_exited() {
                return Ok(());
            }
            self.teardown_worker(false);
        }
        let spawner = self
            .spawner
            .clone()
            .ok_or(PlayerError::WorkerNotFound("worker executable or libmpv"))?;
        let conn = spawner.spawn()?;
        self.next_inc += 1;
        let id = self.next_inc;
        self.shared.inc.store(id, Ordering::Release);
        self.shared.touch();
        let (inc, ready) = link::start(id, conn, &self.shared, &self.surface, self.tx.clone());
        self.shared.set_writer(Some(inc.writer()));
        let verdict = match ready.recv_timeout(self.cfg.timeouts.spawn_ready) {
            Ok(Some(Message::Ready { protocol, .. })) if protocol == PROTOCOL_VERSION => Ok(()),
            Ok(Some(Message::Ready { protocol, .. })) => Err(PlayerError::ProtocolMismatch {
                worker: protocol,
                host: PROTOCOL_VERSION,
            }),
            Ok(_) => Err(PlayerError::SpawnFailed(
                "worker failed to initialize".into(),
            )),
            Err(_) => Err(PlayerError::SpawnFailed("worker start timed out".into())),
        };
        if let Err(e) = verdict {
            self.shared.inc.store(0, Ordering::Release);
            self.shared.set_writer(None);
            inc.kill();
            return Err(e);
        }
        self.inc = Some(inc);
        Ok(())
    }

    /// 현재 워커를 이 핸들로 종료하고 공유 상태·버퍼·대기 명령을 정리한다 (`quit` = 정상 종료 시도)
    pub fn teardown_worker(&mut self, quit: bool) {
        self.shared.epoch.store(0, Ordering::Release);
        self.shared.inc.store(0, Ordering::Release);
        self.shared.set_writer(None);
        if let Some(mut inc) = self.inc.take() {
            if quit {
                let _ = inc.send(Command::Quit, false);
                if !inc.process.wait_exit(self.cfg.timeouts.quit) {
                    inc.process.kill();
                }
            }
            inc.kill();
        }
        if self.cur_epoch != 0 {
            self.surface.retire_bank(self.cur_epoch);
            self.cur_epoch = 0;
        }
        for p in std::mem::take(&mut self.pending) {
            if let Some(r) = p.reply {
                // 죽은 워커에 보낸 명령 — 복원이 설정을 다시 적용하므로 호출자에게는 성공으로 답한다
                let res = if matches!(p.cmd, Command::FrameStep | Command::FrameBackStep) {
                    Err(PlayerError::Restarting)
                } else {
                    Ok(Reply::Unit)
                };
                let _ = r.send(res);
            }
        }
    }

    // ---------- 버퍼 ----------

    fn new_bank(&mut self) -> Result<BankAttach, PlayerError> {
        let (w, h) = self.rem.render_size;
        let handle = self
            .inc
            .as_ref()
            .ok_or(PlayerError::HostGone)?
            .process
            .raw_handle();
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        self.surface
            .create_bank(epoch, w, h, handle)
            .map_err(surface_error)
    }

    /// 새 epoch 버퍼를 만들고 `Attach` 를 보낸다. 성공은 `on_attached`, 실패는 `on_attach_failed` 가 이어받는다
    pub fn attach_new(&mut self, purpose: AttachFor) -> Result<(), PlayerError> {
        let b = self.new_bank()?;
        let old = self.cur_epoch;
        // 새 epoch 의 프레임이 Attach 응답보다 먼저 와도 받아들이도록 먼저 올린다
        self.shared.epoch.store(b.epoch, Ordering::Release);
        self.last_epoch_change = Some(Instant::now());
        let cmd = Command::Attach {
            epoch: b.epoch,
            w: b.w,
            h: b.h,
            stride: b.stride,
            size: b.size,
            handles: b.handles,
        };
        match self.send_pending(
            cmd,
            Done::Attach {
                new: b.epoch,
                old,
                purpose,
            },
            None,
            false,
            self.cfg.timeouts.command,
        ) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.shared.epoch.store(old, Ordering::Release);
                self.surface.retire_bank(b.epoch);
                Err(e)
            }
        }
    }

    pub fn on_attached(&mut self, new: u64, old: u64, purpose: AttachFor) {
        self.cur_epoch = new;
        if old != 0 {
            self.surface.retire_bank(old);
        }
        match purpose {
            AttachFor::Open => {
                if let Phase::Opening(ow) = &mut self.phase {
                    ow.attached = true;
                }
                self.send_open_command();
            }
            AttachFor::Restore => self.send_restore_commands(),
            AttachFor::Resize => {}
        }
    }

    pub fn on_attach_failed(&mut self, new: u64, old: u64, purpose: AttachFor, err: PlayerError) {
        self.shared.epoch.store(old, Ordering::Release);
        self.surface.retire_bank(new);
        match purpose {
            AttachFor::Open => self.fail_open(err),
            AttachFor::Restore => self.worker_failed("attach failed"),
            AttachFor::Resize => {}
        }
    }

    /// 캔버스 크기 변경 → 렌더 크기. 같으면 무시, 250ms 안의 연속 변경은 마지막 값만 적용한다
    pub fn set_render_size(&mut self, w: u32, h: u32) {
        let size = fit_render_size(w, h, RENDER_CAP);
        if size == self.rem.render_size && self.queued_size.is_none() {
            return;
        }
        self.queued_size = Some(size);
        self.apply_queued_resize(Instant::now());
    }

    pub fn apply_queued_resize(&mut self, now: Instant) {
        let Some(size) = self.queued_size else { return };
        if size == self.rem.render_size {
            self.queued_size = None;
            return;
        }
        let live = self.inc.is_some() && matches!(self.phase, Phase::Active);
        if !live {
            // 열려 있지 않거나 복원 중 — 크기만 기억하고 다음 연결에서 쓴다
            self.rem.render_size = size;
            self.queued_size = None;
            return;
        }
        if self
            .last_epoch_change
            .is_some_and(|t| now.saturating_duration_since(t) < self.cfg.timeouts.resize_interval)
        {
            return;
        }
        self.queued_size = None;
        let before = self.rem.render_size;
        self.rem.render_size = size;
        if self.attach_new(AttachFor::Resize).is_err() {
            self.rem.render_size = before;
        }
    }

    // ---------- 열기 ----------

    pub fn open(&mut self, req: OpenRequest, reply: ReplyTx, close_gen: u64) {
        self.cancel_open(PlayerError::Cancelled);
        self.cancel_restart_reply(PlayerError::Cancelled);
        self.streak = 0;
        let usable = self.spawner.is_some() && self.surface.capabilities_ok();
        if !usable {
            let _ = reply.send(Err(PlayerError::WorkerNotFound(
                "worker executable, libmpv or shared buffers",
            )));
            return;
        }
        if let Err(e) = self.ensure_worker() {
            let _ = reply.send(Err(e));
            return;
        }
        // 이전 세션은 새 열기가 대체한다 (워커가 `loadfile replace` 로 닫는다)
        self.session_counter += 1;
        let keep = (self.rem.volume, self.rem.muted, self.rem.render_active);
        self.rem = Remembered {
            session_id: self.session_counter,
            asset_id: req.asset_id,
            path: Some(req.path.clone()),
            paused: !req.autoplay,
            volume: keep.0,
            muted: keep.1,
            render_active: keep.2,
            ranges: req.ranges.clone(),
            duration_hint_ms: req.duration_hint_ms,
            start_ms: req.start_ms,
            render_size: fit_render_size(req.canvas.0, req.canvas.1, RENDER_CAP),
            ..Remembered::default()
        };
        self.ended = false;
        self.queued_size = None;
        self.bridge.reset();
        let opening = {
            let mut e = self.state_event();
            e.state = EventState::Opening;
            e
        };
        self.bridge.emit(opening, Emit::Transition, Instant::now());
        let ow = OpenWait {
            reply,
            deadline: Instant::now() + self.cfg.timeouts.open,
            close_gen,
            attached: false,
            loaded: false,
            restart: None,
            baseline: self.shared.frames.load(Ordering::Acquire),
        };
        self.phase = Phase::Opening(Box::new(ow));
        self.shared.want_frame.store(true, Ordering::Release);
        if let Err(e) = self.attach_new(AttachFor::Open) {
            self.fail_open(e);
        }
    }

    fn send_open_command(&mut self) {
        let Some(path) = self
            .rem
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
        else {
            return;
        };
        let cmd = Command::Open {
            path,
            paused: self.rem.paused,
            ranges: self.rem.ranges.clone(),
            duration_ms: self.rem.duration_hint_ms,
            start_ms: self.rem.start_ms,
        };
        if !self.rem.render_active {
            let _ = self
                .inc
                .as_mut()
                .map(|i| i.send(Command::SetRenderActive { active: false }, false));
        }
        let timeout = self.cfg.timeouts.open;
        if let Err(e) = self.send_pending(cmd, Done::OpenAck, None, false, timeout) {
            self.fail_open(e);
        }
    }

    /// 열기 완료 조건: 로드 + 첫 재시작 + (프레임 게시 | 렌더 꺼짐 | 유예 경과)
    pub fn try_finish_open(&mut self, now: Instant) {
        let grace = self.cfg.timeouts.frame_grace;
        let frames = self.shared.frames.load(Ordering::Acquire);
        let Phase::Opening(ow) = &self.phase else {
            return;
        };
        let Some((time_ms, at)) = ow.restart else {
            return;
        };
        if !(ow.loaded && ow.attached) {
            return;
        }
        let frame = frames > ow.baseline
            || !self.rem.render_active
            || now.saturating_duration_since(at) >= grace;
        if !frame {
            return;
        }
        let Phase::Opening(ow) = std::mem::replace(&mut self.phase, Phase::Active) else {
            return;
        };
        self.rem.pos_ms = time_ms;
        let info = OpenInfo {
            session_id: self.rem.session_id,
            length_ms: self.rem.length_ms,
            fps: self.rem.fps,
            paused: self.rem.paused,
            time_ms,
        };
        let seed = self.state_event();
        self.bridge.seed(seed);
        let _ = ow.reply.send(Ok(Reply::Open(info)));
    }

    /// 열기 실패 — 세션을 접고 버퍼를 놓는다 (워커는 남긴다)
    pub fn fail_open(&mut self, err: PlayerError) {
        if let Phase::Opening(ow) = std::mem::replace(&mut self.phase, Phase::Idle) {
            let _ = ow.reply.send(Err(err));
        }
        self.end_session();
    }

    /// 대기 중인 열기를 취소한다
    pub fn cancel_open(&mut self, err: PlayerError) {
        if matches!(self.phase, Phase::Opening(_)) {
            self.fail_open(err);
        }
    }

    pub fn cancel_restart_reply(&mut self, err: PlayerError) {
        if let Some(r) = self.restart_reply.take() {
            let _ = r.send(Err(err));
        }
    }

    /// 세션 상태를 비운다 (설정 중 음량·숨김 상태는 유지). 이후 이전 세션의 이벤트는 나가지 않는다
    pub fn end_session(&mut self) {
        let keep = (
            self.rem.volume,
            self.rem.muted,
            self.rem.render_active,
            self.rem.render_size,
        );
        self.rem = Remembered {
            volume: keep.0,
            muted: keep.1,
            render_active: keep.2,
            render_size: keep.3,
            ..Remembered::default()
        };
        self.ended = false;
        self.bridge.reset();
        self.queued_size = None;
        if !matches!(self.phase, Phase::Failed) {
            self.phase = Phase::Idle;
        }
        self.shared.want_frame.store(false, Ordering::Release);
        self.shared.epoch.store(0, Ordering::Release);
        if self.cur_epoch != 0 {
            self.surface.retire_bank(self.cur_epoch);
            self.cur_epoch = 0;
        }
    }

    // ---------- 명령 전송 ----------

    /// 응답을 기다리는 명령을 보낸다. `retry` 면 응답이 없을 때 한 번 더 보낸다 (멱등 명령만)
    pub fn send_pending(
        &mut self,
        cmd: Command,
        done: Done,
        reply: Option<ReplyTx>,
        retry: bool,
        timeout: Duration,
    ) -> Result<(), PlayerError> {
        let inc = self.inc.as_mut().ok_or(PlayerError::HostGone)?;
        let id = inc.send(cmd.clone(), true).ok_or(PlayerError::Timeout)?;
        self.pending.push(Pending {
            id,
            cmd,
            done,
            reply,
            deadline: Instant::now() + timeout,
            retry,
            timeout,
        });
        Ok(())
    }

    // ---------- 실패·재시작 ----------

    /// 워커가 죽었거나(파이프 종료·침묵) 응답하지 않아 죽였다
    pub fn worker_failed(&mut self, why: &'static str) {
        self.last_down = Some(why);
        self.teardown_worker(false);
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Opening(ow) => {
                // 로드 전에 죽으면 복원 루프 없이 열기만 실패시킨다 (워커는 다음 열기에서 다시 뜬다)
                let _ = ow.reply.send(Err(if why == "open timed out" {
                    PlayerError::Timeout
                } else {
                    PlayerError::OpenFailed("worker stopped".into())
                }));
                self.end_session();
            }
            Phase::Active | Phase::Restoring(_) => self.schedule_restart(),
            Phase::Backoff { until } => self.phase = Phase::Backoff { until },
            Phase::Failed => self.phase = Phase::Failed,
            Phase::Idle => {}
        }
    }

    fn schedule_restart(&mut self) {
        if !self.rem.has_session() {
            self.phase = Phase::Idle;
            return;
        }
        self.streak += 1;
        if self.streak > MAX_RESTARTS {
            self.fail_for_good();
            return;
        }
        let mut e = self.state_event();
        e.state = EventState::Restarting;
        e.restart = Some(RestartInfo {
            attempt: self.streak,
            max: MAX_RESTARTS,
        });
        self.bridge.emit(e, Emit::Transition, Instant::now());
        let wait = self.cfg.timeouts.backoff[usize::from(self.streak - 1).min(2)];
        self.phase = Phase::Backoff {
            until: Instant::now() + wait,
        };
    }

    fn fail_for_good(&mut self) {
        self.phase = Phase::Failed;
        self.queued_size = None;
        let mut e = self.state_event();
        e.state = EventState::RestartFailed;
        e.error = Some("player worker stopped repeatedly".into());
        e.error_code = Some(PlayerErrorCode::PlayerWorkerCrashed);
        e.restart = Some(RestartInfo {
            attempt: MAX_RESTARTS,
            max: MAX_RESTARTS,
        });
        self.bridge.emit(e, Emit::Transition, Instant::now());
        self.cancel_restart_reply(PlayerError::RestartFailed);
    }

    /// 새 워커를 띄워 버퍼를 다시 연결하고 복원을 시작한다
    pub fn begin_restore(&mut self) {
        let Some(plan) = self.rem.plan() else {
            self.phase = Phase::Idle;
            return;
        };
        let rw = RestoreWait {
            plan,
            deadline: Instant::now() + self.cfg.timeouts.restore,
            attached: false,
            baseline: self.shared.frames.load(Ordering::Acquire),
            restart_at: None,
        };
        self.phase = Phase::Restoring(Box::new(rw));
        if self.ensure_worker().is_err() || self.attach_new(AttachFor::Restore).is_err() {
            self.worker_failed("restart spawn failed");
            return;
        }
        self.shared.want_frame.store(true, Ordering::Release);
    }

    fn send_restore_commands(&mut self) {
        let Phase::Restoring(rw) = &mut self.phase else {
            return;
        };
        rw.attached = true;
        rw.baseline = self.shared.frames.load(Ordering::Acquire);
        let cmds = rw.plan.start();
        if let Some(i) = self.inc.as_mut() {
            for c in cmds {
                let _ = i.send(c, false);
            }
        }
    }

    pub fn try_finish_restore(&mut self, now: Instant) {
        let grace = self.cfg.timeouts.frame_grace;
        let frames = self.shared.frames.load(Ordering::Acquire);
        let Phase::Restoring(rw) = &self.phase else {
            return;
        };
        if !(rw.attached && rw.plan.is_done()) {
            return;
        }
        let frame = frames > rw.baseline
            || !self.rem.render_active
            || rw
                .restart_at
                .is_some_and(|t| now.saturating_duration_since(t) >= grace);
        if !frame {
            return;
        }
        let Phase::Restoring(rw) = std::mem::replace(&mut self.phase, Phase::Active) else {
            return;
        };
        let r = rw.plan.remembered();
        self.rem.paused = r.paused;
        self.rem.pos_ms = r.pos_ms;
        self.streak = 0;
        self.ended = false;
        let manual = self.restart_reply.take();
        if manual.is_some() {
            // 수동 재시작은 새 세션 번호를 받는다 (JS 가 이전 세션 이벤트를 걸러낸다)
            self.session_counter += 1;
            self.rem.session_id = self.session_counter;
        }
        self.emit_state(Emit::Transition);
        if let Some(reply) = manual {
            let info = OpenInfo {
                session_id: self.rem.session_id,
                length_ms: self.rem.length_ms,
                fps: self.rem.fps,
                paused: self.rem.paused,
                time_ms: self.rem.pos_ms,
            };
            let _ = reply.send(Ok(Reply::Open(info)));
        }
    }
}
