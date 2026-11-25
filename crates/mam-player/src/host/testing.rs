//! 시험 보조 — 가짜 워커(프로세스 없이 스레드, 메모리 채널 파이프)와 일반 파일 매핑으로 만든 가짜 프레임 표면.
//! `test-support` 기능이나 `cfg(test)` 에서만 컴파일된다 (릴리스 제외). 실제 워커와도 같은 표면을 쓴다 (`mam-player-worker` 의 live 시험).

use std::collections::HashMap;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::core::PCWSTR;

use super::proc::{RawProcessHandle, WorkerConn, WorkerProcess, WorkerSpawner};
use super::surface::{BankAttach, FrameSurface, SurfaceError};
use crate::PlayerError;
use crate::protocol::{Command, FrameMsgOut, Message, PROTOCOL_VERSION, RenderPath, Request, decode_line, encode_line, stride_for};

// ---------- 가짜 프레임 표면 ----------

struct Section {
    host_handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    size: usize,
}

unsafe impl Send for Section {}

/// 파일 매핑으로 만든 표면 — `create_bank` 는 매핑 3개를 만들고 워커 프로세스로 핸들을 복제한다
pub struct MemSurface {
    banks: Mutex<HashMap<u64, Vec<Section>>>,
    pub frames: Mutex<Vec<FrameMsgOut>>,
    pub created: Mutex<Vec<u64>>,
    pub retired: Mutex<Vec<u64>>,
    pub supported: AtomicBool,
    pub fail_create: AtomicBool,
}

impl MemSurface {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            banks: Mutex::default(),
            frames: Mutex::default(),
            created: Mutex::default(),
            retired: Mutex::default(),
            supported: AtomicBool::new(true),
            fail_create: AtomicBool::new(false),
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn last_frame(&self) -> Option<FrameMsgOut> {
        self.frames.lock().unwrap_or_else(|p| p.into_inner()).last().copied()
    }

    /// 호스트 쪽 뷰에서 슬롯 전체 바이트를 읽는다 (워커가 쓴 픽셀 검증용)
    pub fn read_slot(&self, epoch: u64, slot: usize) -> Option<Vec<u8>> {
        let banks = self.banks.lock().unwrap_or_else(|p| p.into_inner());
        let s = banks.get(&epoch)?.get(slot)?;
        // SAFETY: 매핑이 살아 있는 동안(retire 전) 유효한 읽기 전용 뷰
        Some(unsafe { std::slice::from_raw_parts(s.view.Value as *const u8, s.size) }.to_vec())
    }
}

impl FrameSurface for MemSurface {
    fn create_bank(&self, epoch: u64, w: u32, h: u32, worker_process: RawProcessHandle) -> Result<BankAttach, SurfaceError> {
        if self.fail_create.load(Ordering::Relaxed) {
            return Err(SurfaceError::Failed("create_bank failed (test)".into()));
        }
        let stride = stride_for(w);
        let size = stride as u64 * h as u64;
        let mut sections = Vec::new();
        let mut handles = Vec::new();
        for _ in 0..3 {
            // SAFETY: 익명 페이지 파일 매핑 — 핸들·뷰는 Section 이 소유하고 retire 에서 해제한다
            unsafe {
                let m = CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, (size >> 32) as u32, size as u32, PCWSTR::null())
                    .map_err(|e| SurfaceError::Failed(e.to_string()))?;
                let view = MapViewOfFile(m, FILE_MAP_READ, 0, 0, 0);
                let mut dst = HANDLE::default();
                if worker_process == 0 {
                    handles.push(m.0 as usize as u64); // 가짜 워커(같은 프로세스) — 값만 필요하다
                } else {
                    DuplicateHandle(GetCurrentProcess(), m, HANDLE(worker_process as *mut c_void), &mut dst, 0, false, DUPLICATE_SAME_ACCESS)
                        .map_err(|e| SurfaceError::Failed(format!("DuplicateHandle: {e}")))?;
                    handles.push(dst.0 as usize as u64);
                }
                sections.push(Section { host_handle: m, view, size: size as usize });
            }
        }
        self.banks.lock().unwrap_or_else(|p| p.into_inner()).insert(epoch, sections);
        self.created.lock().unwrap_or_else(|p| p.into_inner()).push(epoch);
        Ok(BankAttach { epoch, w, h, stride, size, handles })
    }

    fn retire_bank(&self, epoch: u64) {
        self.retired.lock().unwrap_or_else(|p| p.into_inner()).push(epoch);
        if let Some(secs) = self.banks.lock().unwrap_or_else(|p| p.into_inner()).remove(&epoch) {
            for s in secs {
                // SAFETY: create_bank 가 만든 뷰·핸들
                unsafe {
                    let _ = UnmapViewOfFile(s.view);
                    let _ = CloseHandle(s.host_handle);
                }
            }
        }
    }

    fn post_frame(&self, msg: &FrameMsgOut) {
        self.frames.lock().unwrap_or_else(|p| p.into_inner()).push(*msg);
    }

    fn capabilities_ok(&self) -> bool {
        self.supported.load(Ordering::Relaxed)
    }
}

// ---------- 메모리 채널 파이프 ----------

pub struct ChanWriter(Sender<Vec<u8>>);

impl Write for ChanWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.send(buf.to_vec()).map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct ChanReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(b) => {
                    self.buf = b;
                    self.pos = 0;
                }
                Err(_) => return Ok(0), // 상대가 닫음 = EOF
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

// ---------- 가짜 워커 ----------

/// 가짜 워커의 동작 조절 손잡이 (시험이 값을 바꿔 크래시·침묵·거절을 만든다)
pub struct FakeScript {
    pub protocol: AtomicU32,
    /// Ready 대신 시작 실패(Error 후 종료)
    pub fail_init: AtomicBool,
    /// Open 을 받으면 응답 없이 죽는다
    pub die_on_open: AtomicBool,
    /// Open 접수는 하지만 FileLoaded 를 보내지 않는다
    pub never_load: AtomicBool,
    /// 이 이름의 명령에는 응답하지 않는다 (예: "seek")
    pub mute_cmd: Mutex<Option<String>>,
    /// 아무 것도 보내지 않는다 (침묵 감시 시험)
    pub go_silent: AtomicBool,
    /// FileLoaded 의 소스 길이
    pub duration_ms: AtomicU64,
    pub start_time_ms: AtomicU64,
    /// 받은 명령 기록
    pub log: Mutex<Vec<Command>>,
    pub spawns: AtomicU32,
    /// 다음 N 번의 spawn 을 실패시킨다
    pub fail_spawns: AtomicU32,
    /// 다음 N 번의 Open 에서 로드 직후 죽는다 (복원 실패 시험)
    pub die_after_load: AtomicU32,
}

impl FakeScript {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            protocol: AtomicU32::new(PROTOCOL_VERSION),
            fail_init: AtomicBool::new(false),
            die_on_open: AtomicBool::new(false),
            never_load: AtomicBool::new(false),
            mute_cmd: Mutex::new(None),
            go_silent: AtomicBool::new(false),
            duration_ms: AtomicU64::new(30_000),
            start_time_ms: AtomicU64::new(0),
            log: Mutex::default(),
            spawns: AtomicU32::new(0),
            fail_spawns: AtomicU32::new(0),
            die_after_load: AtomicU32::new(0),
        })
    }

    pub fn commands(&self) -> Vec<Command> {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn count(&self, pred: impl Fn(&Command) -> bool) -> usize {
        self.commands().iter().filter(|c| pred(c)).count()
    }
}

struct FakeProcess {
    dead: Arc<AtomicBool>,
    id: u32,
}

impl WorkerProcess for FakeProcess {
    fn id(&self) -> u32 {
        self.id
    }

    fn raw_handle(&self) -> RawProcessHandle {
        0
    }

    fn has_exited(&mut self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    fn kill(&mut self) {
        self.dead.store(true, Ordering::Release);
    }

    fn wait_exit(&mut self, timeout: Duration) -> bool {
        let end = std::time::Instant::now() + timeout;
        while !self.has_exited() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.has_exited()
    }
}

pub struct FakeSpawner {
    pub script: Arc<FakeScript>,
}

impl FakeSpawner {
    pub fn new(script: Arc<FakeScript>) -> Arc<Self> {
        Arc::new(Self { script })
    }
}

impl WorkerSpawner for FakeSpawner {
    fn spawn(&self) -> Result<WorkerConn, PlayerError> {
        let n = self.script.spawns.fetch_add(1, Ordering::SeqCst) + 1;
        if self.script.fail_spawns.load(Ordering::SeqCst) > 0 {
            self.script.fail_spawns.fetch_sub(1, Ordering::SeqCst);
            return Err(PlayerError::SpawnFailed("fake spawn failure".into()));
        }
        let (cmd_tx, cmd_rx) = channel::<Vec<u8>>();
        let (evt_tx, evt_rx) = channel::<Vec<u8>>();
        let dead = Arc::new(AtomicBool::new(false));
        let (script, d) = (Arc::clone(&self.script), Arc::clone(&dead));
        std::thread::spawn(move || serve(script, d, cmd_rx, evt_tx));
        Ok(WorkerConn {
            process: Box::new(FakeProcess { dead, id: 10_000 + n }),
            cmd: Box::new(ChanWriter(cmd_tx)),
            evt: Box::new(ChanReader { rx: evt_rx, buf: Vec::new(), pos: 0 }),
        })
    }
}

fn cmd_name(c: &Command) -> &'static str {
    match c {
        Command::Open { .. } => "open",
        Command::SetRanges { .. } => "set_ranges",
        Command::Play => "play",
        Command::Pause => "pause",
        Command::Seek { .. } => "seek",
        Command::FrameStep => "frame_step",
        Command::FrameBackStep => "frame_back_step",
        Command::SetSpeed { .. } => "set_speed",
        Command::SetVolume { .. } => "set_volume",
        Command::SubSet { .. } => "sub_set",
        Command::SetRenderActive { .. } => "set_render_active",
        Command::Attach { .. } => "attach",
        Command::Detach => "detach",
        Command::Release { .. } => "release",
        Command::Probe => "probe",
        Command::Close => "close",
        Command::Quit => "quit",
    }
}

/// 가짜 워커 본체 — 실제 워커와 같은 줄 JSON 프로토콜을 말한다
fn serve(script: Arc<FakeScript>, dead: Arc<AtomicBool>, rx: Receiver<Vec<u8>>, tx: Sender<Vec<u8>>) {
    let send = |m: &Message| {
        if !script.go_silent.load(Ordering::Relaxed)
            && let Ok(l) = encode_line(m)
        {
            let _ = tx.send(l.into_bytes());
        }
    };
    if script.fail_init.load(Ordering::Relaxed) {
        send(&Message::Error { code: crate::protocol::WorkerErrorCode::Internal, msg: "init failed".into() });
        return;
    }
    send(&Message::Ready { mpv_api: 0x2_0000, protocol: script.protocol.load(Ordering::Relaxed), render_path: RenderPath::R1 });
    let mut buf = Vec::<u8>::new();
    let (mut epoch, mut seq, mut time, mut paused) = (0u64, 0u64, 0i64, true);
    let mut size = (0u32, 0u32, 0u32);
    let frame = |epoch: u64, seq: &mut u64, size: (u32, u32, u32)| {
        if epoch != 0 {
            *seq += 1;
            send(&Message::FrameReady { epoch, slot: (*seq % 3) as u32, seq: *seq, w: size.0, h: size.1, stride: size.2, render_done_ms: 1.0 });
        }
    };
    let mut hb = std::time::Instant::now();
    loop {
        if dead.load(Ordering::Acquire) {
            return;
        }
        if hb.elapsed() >= Duration::from_millis(100) {
            hb = std::time::Instant::now();
            send(&Message::Stats { rendered: seq, skipped: 0, ignored: 0, cache_hits: 0 });
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(b) => buf.extend_from_slice(&b),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Some(i) = buf.iter().position(|&c| c == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            let Ok(Request { id, command }) = decode_line::<Request>(&String::from_utf8_lossy(&line)) else { continue };
            script.log.lock().unwrap_or_else(|p| p.into_inner()).push(command.clone());
            let muted = script.mute_cmd.lock().unwrap_or_else(|p| p.into_inner()).as_deref() == Some(cmd_name(&command));
            let reply = |ok: bool| {
                if id != 0 && !muted {
                    send(&Message::Reply { id, ok, code: None, error: (!ok).then(|| "rejected".to_string()), data: None });
                }
            };
            match command {
                Command::Attach { epoch: e, w, h, stride, .. } => {
                    epoch = e;
                    size = (w, h, stride);
                    seq = 0;
                    reply(true);
                    if time > 0 || !paused {
                        frame(epoch, &mut seq, size);
                    }
                }
                Command::Open { paused: p, ranges, .. } => {
                    if script.die_on_open.load(Ordering::Relaxed) {
                        return;
                    }
                    reply(true);
                    paused = p;
                    time = 0;
                    if script.never_load.load(Ordering::Relaxed) {
                        continue;
                    }
                    let _ = ranges;
                    send(&Message::FileLoaded { w: Some(1920), h: Some(1080), fps: Some(30.0), duration_ms: Some(script.duration_ms.load(Ordering::Relaxed) as i64), video_format: Some("h264".into()) });
                    if script.die_after_load.load(Ordering::SeqCst) > 0 {
                        script.die_after_load.fetch_sub(1, Ordering::SeqCst);
                        return;
                    }
                    frame(epoch, &mut seq, size);
                    send(&Message::PlaybackRestart { time_ms: 0 });
                    send(&Message::Paused { paused, time_ms: 0 });
                }
                Command::Seek { ms } => {
                    reply(true);
                    time = ms;
                    frame(epoch, &mut seq, size);
                    send(&Message::PlaybackRestart { time_ms: time });
                }
                Command::Play => {
                    reply(true);
                    paused = false;
                    send(&Message::Paused { paused, time_ms: time });
                }
                Command::Pause => {
                    reply(true);
                    paused = true;
                    send(&Message::Paused { paused, time_ms: time });
                }
                Command::FrameStep | Command::FrameBackStep => {
                    reply(true);
                    paused = true;
                    time = (time + if matches!(command, Command::FrameStep) { 33 } else { -33 }).max(0);
                    frame(epoch, &mut seq, size);
                    send(&Message::PlaybackRestart { time_ms: time });
                    send(&Message::Paused { paused, time_ms: time });
                }
                Command::Close => {
                    reply(true);
                    time = 0;
                }
                Command::Quit => {
                    reply(true);
                    return;
                }
                _ => reply(true),
            }
        }
    }
}
