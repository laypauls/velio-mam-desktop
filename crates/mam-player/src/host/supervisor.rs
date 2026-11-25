//! `PlayerHost` — 앱이 쓰는 동기 API. 모든 호출은 감독자 스레드로 요청을 보내고 제한 시간 안에 응답을 기다린다
//! (호출자는 `spawn_blocking` 스레드 — UI 스레드에서 부르지 않는다). 예외: `release_slots`·`set_render_active`·`close_nowait` 는
//! UI 스레드에서 불러도 막히지 않는다.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use super::core::{Core, Reply, Req, ReqKind};
use super::events::EventSink;
use super::link::{Inbox, Shared};
use super::proc::{ProcessSpawner, WorkerSpawner};
use super::surface::FrameSurface;
use super::types::{HostConfig, OpenInfo, OpenRequest};
use crate::PlayerError;
use crate::protocol::{ControlAction, RangeMs};

pub struct PlayerHost {
    tx: Sender<Inbox>,
    shared: Arc<Shared>,
    surface: Arc<dyn FrameSurface>,
    has_engine_files: bool,
    open_backstop: Duration,
    call_backstop: Duration,
    join: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl PlayerHost {
    /// 워커 실행 파일과 libmpv 폴더가 설정에 있으면 실제 프로세스 스포너를 쓴다. 워커는 첫 `open` 에서 띄운다
    pub fn new(cfg: HostConfig, surface: Arc<dyn FrameSurface>, sink: EventSink) -> Self {
        let spawner: Option<Arc<dyn WorkerSpawner>> = match (&cfg.worker_exe, &cfg.libmpv_dir) {
            (Some(exe), Some(dir)) => Some(Arc::new(ProcessSpawner {
                worker_exe: exe.clone(),
                libmpv_dir: dir.clone(),
                inherit_stderr: cfg.inherit_stderr,
                env: cfg.worker_env.clone(),
            })),
            _ => None,
        };
        Self::with_spawner(cfg, surface, sink, spawner)
    }

    /// 스포너를 직접 지정한다 (시험: 가짜 워커). `None` = 엔진 없음
    pub fn with_spawner(
        cfg: HostConfig,
        surface: Arc<dyn FrameSurface>,
        sink: EventSink,
        spawner: Option<Arc<dyn WorkerSpawner>>,
    ) -> Self {
        let (tx, rx) = channel();
        let shared = Shared::new();
        let has_engine_files = spawner.is_some();
        let (open_t, cmd_t) = (cfg.timeouts.open, cfg.timeouts.command);
        let core = Core::new(
            cfg,
            Arc::clone(&surface),
            spawner,
            Arc::clone(&shared),
            tx.clone(),
            rx,
            sink,
        );
        let join = std::thread::Builder::new()
            .name("mam-player-supervisor".into())
            .spawn(move || core.run())
            .ok();
        let (open_backstop, call_backstop) = (
            open_t + Duration::from_secs(25),
            cmd_t * 2 + Duration::from_secs(12),
        );
        Self {
            tx,
            shared,
            surface,
            has_engine_files,
            open_backstop,
            call_backstop,
            join: std::sync::Mutex::new(join),
        }
    }

    /// 워커·libmpv 파일이 있고 WebView2 가 SharedBuffer 를 지원하는가 — 워커를 띄우지 않는다
    pub fn is_available(&self) -> bool {
        self.has_engine_files && self.surface.capabilities_ok()
    }

    fn call(&self, kind: ReqKind, backstop: Duration) -> Result<Reply, PlayerError> {
        let (rtx, rrx) = channel();
        self.tx
            .send(Inbox::Req(Req {
                kind,
                reply: Some(rtx),
            }))
            .map_err(|_| PlayerError::HostGone)?;
        match rrx.recv_timeout(backstop) {
            Ok(r) => r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(PlayerError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(PlayerError::HostGone),
        }
    }

    fn unit(&self, kind: ReqKind) -> Result<(), PlayerError> {
        self.call(kind, self.call_backstop).map(|_| ())
    }

    fn fire(&self, kind: ReqKind) {
        let _ = self.tx.send(Inbox::Req(Req { kind, reply: None }));
    }

    pub fn open(&self, req: OpenRequest) -> Result<OpenInfo, PlayerError> {
        match self.call(ReqKind::Open(req), self.open_backstop)? {
            Reply::Open(i) => Ok(i),
            Reply::Unit => Err(PlayerError::HostGone),
        }
    }

    pub fn set_ranges(
        &self,
        asset_id: i64,
        ranges: Option<Vec<RangeMs>>,
    ) -> Result<(), PlayerError> {
        self.unit(ReqKind::SetRanges { asset_id, ranges })
    }

    pub fn set_subtitle(&self, asset_id: i64, path: Option<PathBuf>) -> Result<(), PlayerError> {
        self.unit(ReqKind::SetSubtitle { asset_id, path })
    }

    pub fn control(&self, a: ControlAction) -> Result<(), PlayerError> {
        self.unit(ReqKind::Control(a))
    }

    pub fn seek(&self, ms: i64) -> Result<(), PlayerError> {
        self.unit(ReqKind::Seek(ms))
    }

    pub fn set_rate(&self, r: f32) -> Result<(), PlayerError> {
        self.unit(ReqKind::SetRate(f64::from(r)))
    }

    pub fn set_volume(&self, v: i32, muted: bool) -> Result<(), PlayerError> {
        self.unit(ReqKind::SetVolume {
            volume: v.clamp(0, 100) as u8,
            muted,
        })
    }

    pub fn set_render_size(&self, w: u32, h: u32) -> Result<(), PlayerError> {
        self.unit(ReqKind::SetRenderSize(w, h))
    }

    /// UI 스레드에서 불러도 된다 (응답을 기다리지 않는다)
    pub fn set_render_active(&self, active: bool) {
        self.fire(ReqKind::SetRenderActive(active));
    }

    pub fn restart(&self) -> Result<OpenInfo, PlayerError> {
        match self.call(ReqKind::Restart, self.open_backstop)? {
            Reply::Open(i) => Ok(i),
            Reply::Unit => Err(PlayerError::HostGone),
        }
    }

    pub fn close(&self) -> Result<(), PlayerError> {
        self.unit(ReqKind::Close)
    }

    /// 페이지 재로드 훅(UI 스레드) — 대기 중인 열기를 취소하고 닫기를 큐에 넣는다. 응답을 기다리지 않는다
    pub fn close_nowait(&self) {
        self.shared.close_gen.fetch_add(1, Ordering::AcqRel);
        self.fire(ReqKind::Close);
    }

    /// JS 가 돌려준 슬롯들 (UI 스레드) — 절대 막지 않는다. epoch 가 현재가 아니면 버린다
    pub fn release_slots(&self, epoch: u64, slots: &[(u32, u64)]) {
        self.shared.release(epoch, slots);
    }

    /// 현재 프레임을 받아들이는 epoch (UI 핸들러의 검사용). 0 = 없음
    pub fn current_epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }

    /// 앱 종료 — 워커에 `Quit` 를 보내고 `timeout` 까지 기다린 뒤 핸들로 종료한다
    pub fn shutdown(&self, timeout: Duration) {
        let (rtx, rrx) = channel();
        if self
            .tx
            .send(Inbox::Req(Req {
                kind: ReqKind::Shutdown,
                reply: Some(rtx),
            }))
            .is_ok()
        {
            let _ = rrx.recv_timeout(timeout.max(Duration::from_millis(100)));
        }
        let handle = self.join.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(h) = handle
            && h.is_finished()
        {
            let _ = h.join();
        }
    }

    /// 시험 전용 — 워커를 비정상 종료시킨다 (크래시 시뮬레이션)
    #[cfg(any(test, feature = "test-support"))]
    pub fn kill_worker_for_test(&self) {
        let _ = self.unit(ReqKind::KillWorker);
    }

    /// 시험 전용 — 버려진 release 수
    #[cfg(any(test, feature = "test-support"))]
    pub fn dropped_releases(&self) -> u64 {
        self.shared.release_dropped.load(Ordering::Relaxed)
    }
}

impl Drop for PlayerHost {
    fn drop(&mut self) {
        // shutdown 을 부르지 않고 버려져도 감독자가 워커를 정리하도록 요청을 보낸다 (응답은 기다리지 않는다)
        let (rtx, _rrx) = channel();
        let _ = self.tx.send(Inbox::Req(Req {
            kind: ReqKind::Shutdown,
            reply: Some(rtx),
        }));
    }
}

#[cfg(test)]
impl PlayerHost {
    /// 시험 전용 — 워커가 보낸 것처럼 메시지를 주입한다 (`inc` = 세대 번호)
    pub(crate) fn inject(&self, inc: u64, msg: crate::protocol::Message) {
        let _ = self.tx.send(Inbox::Worker {
            inc,
            msg: Some(msg),
        });
    }

    pub(crate) fn current_inc(&self) -> u64 {
        self.shared.inc.load(Ordering::Acquire)
    }
}
