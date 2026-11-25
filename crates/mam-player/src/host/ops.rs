//! 사용자 요청 처리 (감독자 스레드). 규칙(계약 §1, §7):
//! - 열린 세션이 없으면 `Ok` no-op (열기·재시작 제외). 다른 `asset_id` 의 구간·자막 요청은 무시한다
//! - 복원(재시작) 중: seek·배속·음량·구간·자막·재생/정지는 기억만 갱신하고 `Ok`, 프레임 이동은 `Restarting`
//! - 3회 연속 실패 뒤(`Failed`): 열기·재시작·닫기만 받고 나머지는 `RestartFailed`

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use super::core::{Core, Done, Phase, Reply, ReplyTx, Req, ReqKind};
use crate::PlayerError;
use crate::protocol::{Command, ControlAction};

fn answer(r: &Option<ReplyTx>, v: Result<Reply, PlayerError>) {
    if let Some(r) = r {
        let _ = r.send(v);
    }
}

impl Core {
    /// 반환: 감독자 루프를 끝낼 것인가
    pub fn on_req(&mut self, req: Req) -> bool {
        let Req { kind, reply } = req;
        match kind {
            ReqKind::Open(r) => {
                if let Some(rep) = reply {
                    let close_gen = self.shared.close_gen.load(Ordering::Acquire);
                    self.open(r, rep, close_gen);
                }
            }
            ReqKind::Restart => self.restart(reply),
            ReqKind::Close => self.close(reply),
            ReqKind::Shutdown => {
                self.cancel_open(PlayerError::Cancelled);
                self.teardown_worker(true);
                answer(&reply, Ok(Reply::Unit));
                return true;
            }
            ReqKind::SetRenderSize(w, h) => {
                self.set_render_size(w, h);
                answer(&reply, Ok(Reply::Unit));
            }
            ReqKind::SetRenderActive(active) => self.render_active(active, reply),
            ReqKind::SetRanges { asset_id, ranges } => {
                if asset_id != self.rem.asset_id || !self.rem.has_session() {
                    answer(&reply, Ok(Reply::Unit));
                } else {
                    self.rem.ranges = ranges.clone();
                    let hint = self.rem.duration_hint_ms;
                    self.user_cmd(
                        Command::SetRanges {
                            ranges,
                            duration_ms: hint,
                        },
                        true,
                        reply,
                    );
                }
            }
            ReqKind::SetSubtitle { asset_id, path } => {
                if asset_id != self.rem.asset_id || !self.rem.has_session() {
                    answer(&reply, Ok(Reply::Unit));
                } else {
                    self.user_cmd(
                        Command::SubSet {
                            path: path.map(|p: PathBuf| p.to_string_lossy().into_owned()),
                        },
                        true,
                        reply,
                    );
                }
            }
            ReqKind::Control(a) => self.control(a, reply),
            ReqKind::Seek(ms) => {
                let max = self.rem.length_ms.filter(|l| *l > 0);
                let ms = ms.max(0).min(max.unwrap_or(i64::MAX));
                self.user_cmd(Command::Seek { ms }, true, reply);
            }
            ReqKind::SetRate(r) => self.user_cmd(Command::SetSpeed { speed: r }, true, reply),
            ReqKind::SetVolume { volume, muted } => self.user_cmd(
                Command::SetVolume {
                    volume,
                    mute: muted,
                },
                true,
                reply,
            ),
            #[cfg(any(test, feature = "test-support"))]
            ReqKind::KillWorker => {
                if let Some(inc) = self.inc.as_mut() {
                    inc.process.kill();
                }
                answer(&reply, Ok(Reply::Unit));
            }
        }
        false
    }

    /// 사용자 명령 하나 — 단계에 따라 워커로 보내거나 기억만 갱신한다
    fn user_cmd(&mut self, cmd: Command, idempotent: bool, reply: Option<ReplyTx>) {
        if !self.rem.has_session() {
            // 열린 세션이 없다 — 설정(음량)만 기억한다
            if let Command::SetVolume { volume, mute } = &cmd {
                self.rem.volume = *volume;
                self.rem.muted = *mute;
            }
            answer(&reply, Ok(Reply::Unit));
            return;
        }
        match &mut self.phase {
            Phase::Failed => answer(&reply, Err(PlayerError::RestartFailed)),
            Phase::Backoff { .. } | Phase::Restoring(_) => {
                if matches!(cmd, Command::FrameStep | Command::FrameBackStep) {
                    answer(&reply, Err(PlayerError::Restarting));
                    return;
                }
                self.remember(&cmd);
                // 복원이 목표 위치를 이미 확정한 뒤의 seek 는 새 워커에도 바로 보낸다
                if let (Phase::Restoring(rw), Command::Seek { .. }) = (&self.phase, &cmd)
                    && rw.plan.stage == super::restore::Stage::WaitPosition
                    && let Some(i) = self.inc.as_mut()
                {
                    let _ = i.send(cmd.clone(), false);
                }
                answer(&reply, Ok(Reply::Unit));
            }
            Phase::Idle | Phase::Opening(_) | Phase::Active => {
                self.remember(&cmd);
                let timeout = self.cfg.timeouts.command;
                if let Err(e) =
                    self.send_pending(cmd, Done::Reply, reply.clone(), idempotent, timeout)
                {
                    answer(&reply, Err(e));
                }
            }
        }
    }

    /// 사용자 명령을 복원용 기억(`Remembered`, 복원 계획 포함)에 반영한다
    fn remember(&mut self, cmd: &Command) {
        let apply = |r: &mut super::restore::Remembered| match cmd {
            Command::Seek { ms } => r.pos_ms = *ms,
            Command::Play => r.paused = false,
            Command::Pause | Command::FrameStep | Command::FrameBackStep => r.paused = true,
            Command::SetSpeed { speed } => r.speed = *speed,
            Command::SetVolume { volume, mute } => {
                r.volume = *volume;
                r.muted = *mute;
            }
            Command::SubSet { path } => r.subtitle = path.as_ref().map(PathBuf::from),
            Command::SetRanges { ranges, .. } => r.ranges = ranges.clone(),
            Command::SetRenderActive { active } => r.render_active = *active,
            _ => {}
        };
        apply(&mut self.rem);
        if let Phase::Restoring(rw) = &mut self.phase {
            apply(rw.plan.remembered_mut());
        }
    }

    fn control(&mut self, a: ControlAction, reply: Option<ReplyTx>) {
        let (cmd, idempotent) = match a {
            ControlAction::Play => (Command::Play, true),
            ControlAction::Pause => (Command::Pause, true),
            // 토글은 비멱등 — 재시도하면 두 번 뒤집힌다
            ControlAction::Toggle => (
                if self.rem.paused || self.ended {
                    Command::Play
                } else {
                    Command::Pause
                },
                false,
            ),
            ControlAction::StepForward => (Command::FrameStep, false),
            ControlAction::StepBack => (Command::FrameBackStep, false),
        };
        self.user_cmd(cmd, idempotent, reply);
    }

    fn render_active(&mut self, active: bool, reply: Option<ReplyTx>) {
        self.rem.render_active = active;
        if let Phase::Restoring(rw) = &mut self.phase {
            rw.plan.remembered_mut().render_active = active;
        }
        if self.inc.is_some()
            && self.rem.has_session()
            && matches!(self.phase, Phase::Active | Phase::Opening(_))
        {
            let timeout = self.cfg.timeouts.command;
            let _ = self.send_pending(
                Command::SetRenderActive { active },
                Done::Ignore,
                None,
                true,
                timeout,
            );
        }
        answer(&reply, Ok(Reply::Unit));
    }

    /// 수동 재시작 (`player_restart`) — 실패 상태를 풀고 기억한 세션을 새 워커로 복원한다
    fn restart(&mut self, reply: Option<ReplyTx>) {
        let Some(rep) = reply else { return };
        if !self.rem.has_session() {
            let _ = rep.send(Err(PlayerError::OpenFailed("no media to restart".into())));
            return;
        }
        if self.spawner.is_none() || !self.surface.capabilities_ok() {
            let _ = rep.send(Err(PlayerError::WorkerNotFound(
                "worker executable, libmpv or shared buffers",
            )));
            return;
        }
        self.cancel_open(PlayerError::Cancelled);
        self.cancel_restart_reply(PlayerError::Cancelled);
        self.streak = 0;
        self.restart_reply = Some(rep);
        self.teardown_worker(false);
        self.phase = Phase::Idle;
        self.begin_restore();
    }

    /// 닫기 — 워커가 파일을 놓았다는 응답(또는 타임아웃)까지 기다린다. 이후 이전 세션의 이벤트는 나가지 않는다
    fn close(&mut self, reply: Option<ReplyTx>) {
        self.cancel_open(PlayerError::Cancelled);
        self.cancel_restart_reply(PlayerError::Cancelled);
        let had_worker = self.inc.is_some()
            && self.rem.has_session()
            && !matches!(self.phase, Phase::Backoff { .. } | Phase::Failed);
        self.phase = Phase::Idle;
        self.streak = 0;
        self.end_session();
        if !had_worker {
            if self.inc.is_some() {
                let _ = self.inc.as_mut().map(|i| i.send(Command::Close, false));
            }
            answer(&reply, Ok(Reply::Unit));
            return;
        }
        let timeout = self.cfg.timeouts.command;
        if self
            .send_pending(
                Command::Close,
                Done::CloseAck,
                reply.clone(),
                false,
                timeout,
            )
            .is_err()
        {
            answer(&reply, Ok(Reply::Unit));
        }
    }
}
