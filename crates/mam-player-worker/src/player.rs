//! mpv 코어 + 렌더 컨텍스트 + 스레드를 묶은 플레이어, 그리고 명령 처리 (계약 §1.5).
//! 원본 보호: mpv 가 어떤 파일도 쓰지 않도록 옵션으로 막는다 (`protection_options`). 원본은 읽기 전용으로만 연다.

use super::mpv_api::*;
use super::render::{self, Shared};
use super::session::{Pending, Session};
use mam_player::protocol::EdlMap;
use mam_player::protocol::{
    Command, Landing, Message, ProbeData, RangeMs, RenderPath, SlotRef, WorkerErrorCode,
    back_cache_frames, build_edl, is_long_gop, volume_to_mpv,
};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 뒤로 이동 캐시 메모리 상한 (D6)
pub const BACK_CACHE_BYTES: u64 = 256 * 1024 * 1024;
/// 구간 재열기 전환 제한 — 넘으면 현재 상태로 마무리한다
const RELOAD_LIMIT: Duration = Duration::from_secs(5);

pub type WErr = (WorkerErrorCode, String);

fn err(code: WorkerErrorCode, msg: impl Into<String>) -> WErr {
    (code, msg.into())
}

/// 관찰 속성 id
pub const OBS_PAUSE: u64 = 1;
pub const OBS_EOF: u64 = 2;
pub const OBS_TIME: u64 = 5;
pub const OBS_CACHE_PAUSE: u64 = 6;

/// 워커 시작 옵션 (개발·시험용 손잡이는 환경변수에서 온다 — 제품 경로는 기본값)
pub struct Options {
    pub log: bool,
    /// mpv `--ao` (시험은 `null` 로 소리를 끈다)
    pub ao: Option<String>,
    /// `Some(true)` = R1(SW) 강제, `Some(false)` = R2 강제, `None` = R2 시도 후 R1 대체
    pub force_sw: Option<bool>,
}

/// 원본·사용자 PC 를 건드리지 않게 하는 고정 옵션 + 재생 기본값 (D7). 순수 — 시험이 목록을 검증한다
pub fn base_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("vo", "libmpv"),
        ("idle", "yes"),
        // 재생 기본값
        ("profile", "fast"),
        ("hr-seek", "yes"),
        ("keep-open", "yes"),
        ("audio-delay", "0.011"),
        ("audio-pitch-correction", "yes"),
        // 원본 보호: 어떤 것도 쓰지 않고, 외부 스크립트·설정·네트워크 도구를 읽지 않는다
        ("config", "no"),
        ("load-scripts", "no"),
        ("ytdl", "no"),
        ("osc", "no"),
        ("input-default-bindings", "no"),
        ("input-builtin-bindings", "no"),
        ("input-vo-keyboard", "no"),
        ("save-position-on-quit", "no"),
        ("write-filename-in-watch-later-config", "no"),
        ("resume-playback", "no"),
        ("cache-on-disk", "no"),
        ("sub-auto", "no"),
        ("audio-file-auto", "no"),
        ("cover-art-auto", "no"),
        ("autocreate-playlist", "no"),
        // 셰이더·ICC 캐시가 사용자 프로필 폴더에 파일을 만들지 않게 한다
        ("gpu-shader-cache", "no"),
        ("icc-cache", "no"),
    ]
}

pub struct Inner {
    pub tx: Sender<Message>,
    pub mpv: Arc<Mpv>,
    pub shared: Arc<Shared>,
    pub st: Mutex<Session>,
    pub render_path: RenderPath,
}

pub struct Player {
    pub inner: Arc<Inner>,
    ctx: SendPtr,
    render_thread: Option<JoinHandle<()>>,
    event_thread: Option<JoinHandle<()>>,
}

impl Inner {
    pub fn lock(&self) -> MutexGuard<'_, Session> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn send(&self, m: Message) {
        let _ = self.tx.send(m);
    }

    /// 캐시 따라잡기 상태를 버린다 — seek·재생·재열기·닫기가 위치를 새로 정하므로 이전 따라잡기의 재시작 신호를 기다리지 않는다
    fn drop_catch_up(&self) {
        self.shared.suppress.store(0, Ordering::Release);
        self.lock().queued_steps.clear();
    }

    fn clear_cache(&self) {
        if let Some(c) = self
            .shared
            .cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            c.clear();
        }
    }

    /// 장GOP 코덱이고 슬롯이 연결돼 있으면 렌더 크기에 맞는 장수로 뒤로 이동 캐시를 켠다 (아니면 끈다)
    pub fn configure_cache(&self) {
        let long = self.lock().video_format.as_deref().is_some_and(is_long_gop);
        let bank = self
            .shared
            .bank
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let frames = match (&bank, long) {
            (Some(b), true) => b.targets.first().map_or(0, |t| {
                back_cache_frames(BACK_CACHE_BYTES, t.stride as u32, t.h)
            }),
            _ => 0,
        };
        *self.shared.cache.lock().unwrap_or_else(|p| p.into_inner()) =
            (frames > 0).then(|| super::cache::FrameCache::new(frames));
    }

    fn load(&self, target: &str) -> Result<(), WErr> {
        self.mpv
            .command(&["loadfile", target, "replace"])
            .map_err(|e| err(WorkerErrorCode::OpenFailed, e))
    }

    fn require_open(&self) -> Result<(), WErr> {
        if self.lock().open {
            Ok(())
        } else {
            Err(err(WorkerErrorCode::NoMedia, "no media is open"))
        }
    }

    fn internal(e: String) -> WErr {
        err(WorkerErrorCode::Internal, e)
    }

    // ---------- 열기 ----------

    /// `start_ms` = 이어보기 시작 위치(소스 시각, 첫 프레임 = 0). 일반 파일은 첫 프레임 pts 를 읽은 직후(프레임을 게시하기 전)
    /// 그 위치로 이동한 뒤 열기를 끝내 0 초 프레임이 보이지 않는다. 구간(EDL) 열기는 기존 열기 중 seek 보류로 처리한다
    pub fn open(
        &self,
        path: String,
        paused: bool,
        ranges: Option<Vec<RangeMs>>,
        hint: Option<i64>,
        start_ms: Option<i64>,
    ) -> Result<(), WErr> {
        if path.is_empty() || path.contains('\0') {
            return Err(err(WorkerErrorCode::InvalidArgument, "invalid path"));
        }
        let edl = match ranges.filter(|r| !r.is_empty()) {
            Some(r) => Some(
                EdlMap::new(&r)
                    .ok_or_else(|| err(WorkerErrorCode::InvalidArgument, "invalid ranges"))?,
            ),
            None => None,
        };
        self.drop_catch_up();
        self.clear_cache();
        self.shared.has_video.store(false, Ordering::Release);
        self.mpv.set_prop("speed", "1").map_err(Self::internal)?;
        // 열기는 항상 정지로 시작한다 — 첫 프레임 pts(start)를 재생이 흐르기 전에 정확히 읽기 위해서이고, 자동 재생은 첫 재시작 뒤에 건다
        self.mpv.set_prop("pause", "yes").map_err(Self::internal)?;
        let probe = edl.is_some();
        let start = start_ms.filter(|ms| *ms > 0);
        let hold_for_start = start.is_some() && !probe;
        {
            let mut s = self.lock();
            *s = Session::begin(path.clone(), edl, hint, paused);
            s.pending = probe.then_some(Pending::Probe);
            if probe {
                s.deferred_seek = start;
            } else {
                s.open_seek = start;
            }
        }
        self.shared
            .hold
            .store(probe || hold_for_start, Ordering::Release);
        self.load(&path)
    }

    /// 구간 단계 2: 첫 프레임 pts 를 얻은 뒤 EDL 로 다시 연다 (이벤트 스레드가 부른다)
    pub fn load_edl(&self) -> Result<(), WErr> {
        self.lock().loaded = false; // EDL 로드의 FILE_LOADED 를 다시 기다린다
        let (path, ranges, start) = {
            let s = self.lock();
            (
                s.path.clone(),
                s.ranges().map(<[RangeMs]>::to_vec).unwrap_or_default(),
                s.start_s,
            )
        };
        self.load(&build_edl(&path, &ranges, (start * 1000.0).round() as i64))
    }

    /// 열린 채 구간 교체 — 현재 소스 위치와 재생 상태를 유지한 채 EDL(또는 일반 파일)로 다시 연다
    pub fn set_ranges(&self, ranges: Option<Vec<RangeMs>>, hint: Option<i64>) -> Result<(), WErr> {
        self.require_open()?;
        let edl = match ranges.filter(|r| !r.is_empty()) {
            Some(r) => Some(
                EdlMap::new(&r)
                    .ok_or_else(|| err(WorkerErrorCode::InvalidArgument, "invalid ranges"))?,
            ),
            None => None,
        };
        let (path, edl_str) = {
            let mut s = self.lock();
            if s.pending.is_some() {
                return Err(err(WorkerErrorCode::Internal, "busy"));
            }
            let pos_ms = s.shown_ms;
            if hint.is_some() {
                s.source_duration_ms = hint;
            }
            s.set_edl(edl);
            s.eof = false;
            let t = s.seek_target(pos_ms);
            let past = t.landing == Landing::PastEnd;
            let resume = !s.paused && !past;
            s.pending = Some(Pending::Reload {
                target_s: t.mpv_s,
                resume_play: resume,
                seeked: false,
                past_end: past,
                deadline: Instant::now() + RELOAD_LIMIT,
            });
            let edl_str = s
                .edl
                .as_ref()
                .map(|e| build_edl(&s.path, e.ranges(), (s.start_s * 1000.0).round() as i64));
            (s.path.clone(), edl_str)
        };
        self.drop_catch_up();
        self.clear_cache();
        self.shared.hold.store(true, Ordering::Release);
        self.mpv.set_prop("pause", "yes").map_err(Self::internal)?;
        self.load(edl_str.as_deref().unwrap_or(&path))
    }

    // ---------- 재생 제어 ----------

    pub fn play(&self) -> Result<(), WErr> {
        self.require_open()?;
        let (eof, rewind, pending) = {
            let mut s = self.lock();
            s.paused = false;
            let pending = s.pending.is_some();
            if let Some(Pending::Reload { resume_play, .. }) = s.pending.as_mut() {
                *resume_play = true;
            }
            if s.opening {
                // 열기 중의 재생 요청 — 첫 프레임이 올라온 뒤 재생한다
                s.autoplay_after_open = true;
                return Ok(());
            }
            (s.eof, s.rewind_target(), pending)
        };
        if pending {
            return Ok(());
        }
        if eof {
            self.drop_catch_up();
            // 끝난 뒤 재생: 처음(첫 구간 시작 또는 파일 시작)으로 돌아가 재생한다
            self.clear_cache();
            self.mpv
                .command(&["seek", &format!("{rewind}"), "absolute+exact"])
                .map_err(Self::internal)?;
        }
        self.mpv.set_prop("pause", "no").map_err(Self::internal)
    }

    pub fn pause(&self) -> Result<(), WErr> {
        self.require_open()?;
        {
            let mut s = self.lock();
            s.paused = true;
            if let Some(Pending::Reload { resume_play, .. }) = s.pending.as_mut() {
                *resume_play = false;
            }
            if s.opening {
                s.autoplay_after_open = false;
                return Ok(());
            }
            if s.pending.is_some() {
                return Ok(());
            }
        }
        self.mpv.set_prop("pause", "yes").map_err(Self::internal)
    }

    pub fn seek(&self, ms: i64) -> Result<(), WErr> {
        self.require_open()?;
        let target = {
            let mut s = self.lock();
            let t = s.seek_target(ms);
            let opening = s.opening;
            match s.pending.as_mut() {
                Some(Pending::Reload {
                    target_s, past_end, ..
                }) => {
                    *target_s = t.mpv_s;
                    *past_end = t.landing == Landing::PastEnd;
                    return Ok(());
                }
                Some(_) => return Ok(()), // 구간 열기 1·2단계 — 열기가 끝난 뒤 다시 보내야 한다
                None => {}
            }
            if opening {
                s.deferred_seek = Some(ms);
                return Ok(());
            }
            t
        };
        self.drop_catch_up();
        self.clear_cache();
        self.mpv
            .command(&["seek", &format!("{}", target.mpv_s), "absolute+exact"])
            .map_err(Self::internal)
    }

    pub fn frame_step(&self) -> Result<(), WErr> {
        self.require_open()?;
        {
            let mut s = self.lock();
            if s.pending.is_some() || s.opening {
                return Ok(());
            }
            if self.shared.suppress.load(Ordering::Acquire) > 0 {
                if s.queued_steps.len() < 16 {
                    s.queued_steps.push_back(true);
                }
                return Ok(());
            }
            s.paused = true;
            s.step = Some(Instant::now());
        }
        self.mpv.set_prop("pause", "yes").map_err(Self::internal)?;
        self.mpv.command(&["frame-step"]).map_err(Self::internal)
    }

    pub fn frame_back_step(&self) -> Result<(), WErr> {
        self.require_open()?;
        {
            let mut s = self.lock();
            if s.pending.is_some() || s.opening {
                return Ok(());
            }
            if self.shared.suppress.load(Ordering::Acquire) > 0 {
                if s.queued_steps.len() < 16 {
                    s.queued_steps.push_back(false);
                }
                return Ok(());
            }
            s.paused = true;
            s.step = Some(Instant::now());
        }
        self.mpv.set_prop("pause", "yes").map_err(Self::internal)?;
        // 캐시에 이전 프레임이 있으면 바로 보여 주고 mpv 도 같은 위치로 따라가게 하되 그 결과 프레임은 내보내지 않는다
        if let Some(ms) = self.serve_back_from_cache() {
            self.shared.suppress.fetch_add(1, Ordering::AcqRel);
            *self
                .shared
                .suppress_until
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Instant::now() + Duration::from_secs(10);
            {
                let mut s = self.lock();
                s.shown_ms = ms;
                s.step = None;
            }
            self.send(Message::PlaybackRestart { time_ms: ms });
            self.send(Message::Paused {
                paused: true,
                time_ms: ms,
            });
        }
        self.mpv
            .command(&["frame-back-step"])
            .map_err(Self::internal)
    }

    /// 뒤로 한 프레임을 캐시로 처리한다. 적중하면 FREE 슬롯에 그 프레임을 복사해 게시하고 새 소스 시각(ms)을 돌려준다
    fn serve_back_from_cache(&self) -> Option<i64> {
        let bank = self
            .shared
            .bank
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()?;
        let (fps, shown) = {
            let s = self.lock();
            (s.fps.filter(|f| f.is_finite() && *f > 0.0)?, s.shown_ms)
        };
        let k = (shown as f64 / 1000.0 * fps).round();
        if k < 1.0 {
            return None; // 첫 프레임 — 이전 프레임이 없다
        }
        let mut guard = self.shared.cache.lock().unwrap_or_else(|p| p.into_inner());
        let cache = guard.as_mut()?;
        if cache.len() < 2 {
            return None;
        }
        let slot = bank
            .table
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .acquire()?;
        let t = bank.targets[slot];
        let Some(e) = cache.step_back() else {
            bank.table
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .abort(slot);
            return None;
        };
        if e.w != t.w || e.h != t.h || e.stride != t.stride {
            // 렌더 크기가 바뀌어 이력을 쓸 수 없다
            bank.table
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .abort(slot);
            cache.clear();
            return None;
        }
        // SAFETY: 슬롯은 WRITING 상태로 이 스레드만 쓴다. e.data 길이 = stride*h
        unsafe { std::ptr::copy_nonoverlapping(e.data.as_ptr(), t.ptr, e.data.len().min(t.len())) };
        let done = super::bank::now_epoch_ms();
        let seq = bank
            .table
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .publish(slot)?;
        self.shared.rendered.fetch_add(1, Ordering::Relaxed);
        self.send(Message::FrameReady {
            epoch: bank.epoch,
            slot: slot as u32,
            seq,
            w: t.w,
            h: t.h,
            stride: t.stride as u32,
            render_done_ms: done,
        });
        Some(mam_player::protocol::ms_from_secs((k - 1.0) / fps))
    }

    /// 따라잡기가 끝났으면 대기 중인 프레임 이동을 하나 실행한다 (이벤트 스레드가 부른다)
    pub fn drain_steps(&self) {
        if self.shared.suppress.load(Ordering::Acquire) > 0 {
            return;
        }
        let next = self.lock().queued_steps.pop_front();
        match next {
            Some(true) => drop(self.frame_step()),
            Some(false) => drop(self.frame_back_step()),
            None => {}
        }
    }

    pub fn set_volume(&self, volume: u8, mute: bool) -> Result<(), WErr> {
        self.mpv
            .set_prop_double("volume", volume_to_mpv(volume))
            .map_err(Self::internal)?;
        self.mpv
            .set_prop("mute", if mute || volume == 0 { "yes" } else { "no" })
            .map_err(Self::internal)
    }

    pub fn set_speed(&self, speed: f64) -> Result<(), WErr> {
        if !speed.is_finite() || !(0.1..=16.0).contains(&speed) {
            return Err(err(WorkerErrorCode::InvalidArgument, "invalid speed"));
        }
        self.mpv
            .set_prop_double("speed", speed)
            .map_err(Self::internal)
    }

    pub fn sub_set(&self, path: Option<String>) -> Result<(), WErr> {
        self.require_open()?;
        {
            let mut s = self.lock();
            if s.pending.is_some() || s.opening {
                // 파일을 바꾸는 중이라 지금 붙일 수 없다 — 끝난 뒤 적용한다
                s.deferred_sub = Some(path);
                return Ok(());
            }
        }
        match path {
            Some(p) => {
                if p.is_empty() || p.contains('\0') {
                    return Err(err(
                        WorkerErrorCode::InvalidArgument,
                        "invalid subtitle path",
                    ));
                }
                self.mpv
                    .command(&["sub-add", &p, "select"])
                    .map_err(Self::internal)?;
                self.mpv
                    .set_prop("sub-visibility", "yes")
                    .map_err(Self::internal)
            }
            None => {
                // 추가한 자막을 지운다 (없으면 오류가 나도 괜찮다)
                let _ = self.mpv.command(&["sub-remove"]);
                self.mpv.set_prop("sid", "no").map_err(Self::internal)
            }
        }
    }

    pub fn set_render_active(&self, active: bool) {
        self.shared.render_active.store(active, Ordering::Release);
        if active {
            self.shared.redraw.store(true, Ordering::Release);
        }
        self.shared.notify();
    }

    pub fn attach(
        &self,
        epoch: u64,
        w: u32,
        h: u32,
        stride: u32,
        size: u64,
        handles: &[u64],
    ) -> Result<(), WErr> {
        // 먼저 새 뱅크를 만들어 성공한 뒤에 교체한다 (실패해도 기존 연결 유지)
        let bank = super::bank::Bank::attach(epoch, w, h, stride, size, handles)
            .map_err(|e| err(WorkerErrorCode::InvalidArgument, e))?;
        *self.shared.bank.lock().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(bank));
        self.shared.redraw.store(true, Ordering::Release);
        self.shared.notify();
        self.configure_cache();
        Ok(())
    }

    pub fn release(&self, epoch: u64, slots: &[SlotRef]) {
        if let Some(b) = self
            .shared
            .bank
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            let mut t = b.table.lock().unwrap_or_else(|p| p.into_inner());
            for s in slots {
                if t.release(epoch, s.slot as usize, s.seq)
                    != mam_player::protocol::Outcome::Applied
                {
                    self.shared.ignored.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn close(&self) -> Result<(), WErr> {
        self.drop_catch_up();
        self.clear_cache();
        self.shared.hold.store(false, Ordering::Release);
        self.shared.has_video.store(false, Ordering::Release);
        *self.lock() = Session::default();
        self.mpv.command(&["stop"]).map_err(Self::internal)
    }

    pub fn probe(&self) -> ProbeData {
        let m = &self.mpv;
        ProbeData {
            time_pos: m.get_double("time-pos"),
            paused: m.get_flag("pause"),
            hwdec: m.get_string("hwdec-current"),
            speed: m.get_double("speed"),
            volume: m.get_double("volume"),
            mute: m.get_flag("mute"),
            audio_delay: m.get_double("audio-delay"),
            video_format: m.get_string("video-format"),
            back_cache_len: self
                .shared
                .cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
                .map_or(0, |c| c.len() as u64),
            edl: self.lock().edl.is_some(),
        }
    }
}

impl Player {
    pub fn start(api: Arc<Api>, opts: &Options, tx: Sender<Message>) -> Result<Self, String> {
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err("mpv_create failed".into());
        }
        let mpv = Arc::new(Mpv {
            api: api.clone(),
            handle: SendPtr(handle),
        });
        for (k, v) in base_options() {
            mpv.set_option(k, v)?;
        }
        if let Some(ao) = &opts.ao {
            mpv.set_option("ao", ao)?;
        }
        let r = unsafe { (api.initialize)(handle) };
        if r < 0 {
            return Err(format!("mpv_initialize failed: {}", mpv.err_text(r)));
        }
        if opts.log {
            unsafe { (api.request_log_messages)(handle, c"warn".as_ptr()) };
        }
        for (id, name, fmt) in [
            (OBS_PAUSE, "pause", FORMAT_FLAG),
            (OBS_EOF, "eof-reached", FORMAT_FLAG),
            (OBS_TIME, "time-pos", FORMAT_DOUBLE),
            (OBS_CACHE_PAUSE, "paused-for-cache", FORMAT_FLAG),
        ] {
            mpv.observe(id, name, fmt)?;
        }
        // 렌더 경로: R2(D3D11 + d3d11va) 를 먼저, 실패하면 R1(SW). 하드웨어 디코더가 없는 코덱은 mpv 가 알아서 SW 디코드로 내려간다
        let want_d3d = opts.force_sw != Some(true);
        let d3d = if want_d3d {
            match super::d3d::D3d::new() {
                Ok(d) => Some(d),
                Err(e) if opts.force_sw == Some(false) => return Err(e),
                Err(_) => None,
            }
        } else {
            None
        };
        let mut render_path = RenderPath::R1;
        let ctx = match &d3d {
            Some(d) => match render::create_context_d3d11(&api, handle, d.device_ptr()) {
                Ok(c) => {
                    render_path = RenderPath::R2;
                    c
                }
                Err(e) if opts.force_sw == Some(false) => return Err(e),
                Err(_) => render::create_context(&api, handle)?,
            },
            None => render::create_context(&api, handle)?,
        };
        let shared = Shared::new();
        if render_path == RenderPath::R2 {
            *shared.d3d.lock().unwrap_or_else(|p| p.into_inner()) = d3d;
            mpv.set_prop("hwdec", "d3d11va")?;
        } else {
            mpv.set_prop("hwdec", "no")?;
        }
        // 콜백 컨텍스트는 `shared` 의 Arc 가 살아있는 동안 유효 (free 는 shutdown 에서 콜백 해제 뒤)
        unsafe {
            (api.render_context_set_update_callback)(
                ctx.0,
                Some(render::on_update),
                Arc::as_ptr(&shared) as *mut _,
            )
        };
        let render_thread =
            render::spawn_render_thread(api.clone(), ctx, shared.clone(), tx.clone());
        let inner = Arc::new(Inner {
            tx,
            mpv,
            shared,
            st: Mutex::new(Session::default()),
            render_path,
        });
        let event_thread = super::events::spawn(inner.clone(), opts.log);
        Ok(Self {
            inner,
            ctx,
            render_thread: Some(render_thread),
            event_thread: Some(event_thread),
        })
    }

    pub fn api_version(&self) -> u64 {
        unsafe { (self.inner.mpv.api.client_api_version)() as u64 }
    }

    /// 명령 하나 실행. 반환: Probe 만 데이터를 돌려준다
    pub fn handle(&self, cmd: Command) -> Result<Option<ProbeData>, WErr> {
        let i = &self.inner;
        match cmd {
            Command::Open {
                path,
                paused,
                ranges,
                duration_ms,
                start_ms,
            } => i.open(path, paused, ranges, duration_ms, start_ms)?,
            Command::SetRanges {
                ranges,
                duration_ms,
            } => i.set_ranges(ranges, duration_ms)?,
            Command::Play => i.play()?,
            Command::Pause => i.pause()?,
            Command::Seek { ms } => i.seek(ms)?,
            Command::FrameStep => i.frame_step()?,
            Command::FrameBackStep => i.frame_back_step()?,
            Command::SetSpeed { speed } => i.set_speed(speed)?,
            Command::SetVolume { volume, mute } => i.set_volume(volume, mute)?,
            Command::SubSet { path } => i.sub_set(path)?,
            Command::SetRenderActive { active } => i.set_render_active(active),
            Command::Attach {
                epoch,
                w,
                h,
                stride,
                size,
                handles,
            } => i.attach(epoch, w, h, stride, size, &handles)?,
            Command::Detach => *i.shared.bank.lock().unwrap_or_else(|p| p.into_inner()) = None,
            Command::Release { epoch, slots } => i.release(epoch, &slots),
            Command::Probe => return Ok(Some(i.probe())),
            Command::Close => i.close()?,
            Command::Quit => {}
        }
        Ok(None)
    }

    /// 종료 순서: 렌더 스레드 → 렌더 컨텍스트 → 이벤트 스레드 → mpv 코어 (렌더 컨텍스트는 코어보다 먼저 해제)
    pub fn shutdown(mut self) {
        let i = self.inner.clone();
        i.shared.stop.store(true, Ordering::Release);
        i.shared.notify();
        if let Some(t) = self.render_thread.take() {
            let _ = t.join();
        }
        let api = &i.mpv.api;
        unsafe {
            (api.render_context_set_update_callback)(self.ctx.0, None, std::ptr::null_mut());
            (api.render_context_free)(self.ctx.0);
            (api.wakeup)(i.mpv.handle.0);
        }
        if let Some(t) = self.event_thread.take() {
            let _ = t.join();
        }
        unsafe { (api.terminate_destroy)(i.mpv.handle.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Business rule(원본 보호 옵션 목록) / Validation

    #[test]
    fn t_player_orig_01_protection_options_cover_every_write_path() {
        let o = base_options();
        let get = |k: &str| o.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        // 쓰기 경로 차단 — 하나라도 빠지면 원본 폴더나 사용자 프로필에 파일이 생길 수 있다
        for (k, v) in [
            ("save-position-on-quit", "no"),
            ("write-filename-in-watch-later-config", "no"),
            ("resume-playback", "no"),
            ("config", "no"),
            ("load-scripts", "no"),
            ("input-default-bindings", "no"),
            ("input-builtin-bindings", "no"),
            ("ytdl", "no"),
            ("sub-auto", "no"),
            ("audio-file-auto", "no"),
            ("cover-art-auto", "no"),
            ("autocreate-playlist", "no"),
            ("cache-on-disk", "no"),
            ("gpu-shader-cache", "no"),
            ("icc-cache", "no"),
        ] {
            assert_eq!(get(k), Some(v), "{k}");
        }
        // 제어용 소켓·스크린샷 경로는 아예 열지 않는다
        assert!(o.iter().all(|(k, _)| *k != "input-ipc-server"
            && *k != "screenshot-directory"
            && *k != "log-file"));
    }

    #[test]
    fn t_player_worker_opt_01_playback_defaults_follow_the_contract() {
        let o = base_options();
        let get = |k: &str| o.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        assert_eq!(get("profile"), Some("fast"));
        assert_eq!(get("hr-seek"), Some("yes"));
        assert_eq!(get("keep-open"), Some("yes"));
        assert_eq!(get("audio-delay"), Some("0.011"));
        assert_eq!(get("audio-pitch-correction"), Some("yes"));
        let mut keys: Vec<_> = o.iter().map(|(k, _)| *k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), o.len(), "중복 옵션 없음");
    }
}
