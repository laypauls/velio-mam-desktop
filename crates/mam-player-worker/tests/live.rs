//! 실제 워커 프로세스 + 실제 libmpv 로 감독자(`PlayerHost`)를 시험한다 — 프레임 표면만 파일 매핑 가짜(`MemSurface`)다.
//! 환경: `MAM_LIBMPV_DIR`(없으면 개발 저장소의 `temp/poc-mpv/lib`), 시료 `MAM_PLAYER_SAMPLE_DIR`(없으면 `temp/poc-mpv/samples`).
//! 없으면 `SKIP` 을 출력하고 통과한다. 원본 시료는 임시 폴더로 복사해서만 쓴다 (원본 접근 금지).
//! 분류: Happy / Business rule(EDL·재시작·렌더 꺼짐) / External(워커 강제 종료) / 원본 보호

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use mam_player::PlayerError;
use mam_player::host::testing::MemSurface;
use mam_player::host::{EventState, HostConfig, OpenInfo, OpenRequest, PlayerEvent, PlayerHost, Timeouts};
use mam_player::protocol::ControlAction;

const WORKER: &str = env!("CARGO_BIN_EXE_mam-player-worker");

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn libmpv_dir() -> Option<PathBuf> {
    let d = std::env::var_os("MAM_LIBMPV_DIR").map(PathBuf::from).unwrap_or_else(|| repo().join("temp/poc-mpv/lib"));
    d.join("libmpv-2.dll").is_file().then_some(d)
}

fn samples_dir() -> Option<PathBuf> {
    let d = std::env::var_os("MAM_PLAYER_SAMPLE_DIR").map(PathBuf::from).unwrap_or_else(|| repo().join("temp/poc-mpv/samples"));
    d.join("h264_1080p30.mp4").is_file().then_some(d)
}

static SERIAL: Mutex<()> = Mutex::new(());

struct Live {
    /// 실제 워커·GPU 를 쓰므로 이 파일의 시험은 한 번에 하나씩 돌린다 (과부하에서 시간 의존 단언이 흔들리지 않게)
    _serial: std::sync::MutexGuard<'static, ()>,
    host: Arc<PlayerHost>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    surface: Arc<MemSurface>,
    events: Arc<Mutex<Vec<PlayerEvent>>>,
    media: tempfile::TempDir,
    data: tempfile::TempDir,
    samples: PathBuf,
}

fn live(name: &str) -> Option<Live> {
    live_env(name, vec![])
}

fn live_env(name: &str, extra: Vec<(String, String)>) -> Option<Live> {
    let (Some(mpv), Some(samples)) = (libmpv_dir(), samples_dir()) else {
        println!("SKIP {name}: libmpv (MAM_LIBMPV_DIR) or samples not available");
        return None;
    };
    let serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let surface = MemSurface::new();
    let events: Arc<Mutex<Vec<PlayerEvent>>> = Arc::default();
    let ev = Arc::clone(&events);
    let cfg = HostConfig {
        worker_exe: Some(PathBuf::from(WORKER)),
        libmpv_dir: Some(mpv),
        worker_env: [vec![("MAM_PLAYER_AO".into(), "null".into())], extra].concat(), 
        timeouts: Timeouts { silence: Duration::from_secs(8), ..Timeouts::default() },
        ..HostConfig::default()
    };
    let host = Arc::new(PlayerHost::new(cfg, surface.clone(), Arc::new(move |e| ev.lock().unwrap().push(e))));
    // JS 흉내: 게시된 프레임을 곧바로 돌려준다 (돌려주지 않으면 슬롯 3개가 차서 워커가 건너뛴다)
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (h, s, stop) = (Arc::clone(&host), surface.clone(), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut done = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let frames = s.frames.lock().unwrap().clone();
                for f in frames.iter().skip(done) {
                    h.release_slots(f.epoch, &[(f.slot, f.seq)]);
                }
                done = frames.len();
                std::thread::sleep(Duration::from_millis(4));
            }
        });
    }
    Some(Live { _serial: serial, host, stop, surface, events, media: tempfile::tempdir().unwrap(), data: tempfile::tempdir().unwrap(), samples })
}

impl Live {
    /// 시료를 임시 미디어 폴더로 복사한다 (원본은 건드리지 않는다)
    fn copy(&self, name: &str) -> PathBuf {
        let dst = self.media.path().join(name);
        std::fs::copy(self.samples.join(name), &dst).unwrap();
        dst
    }

    fn open(&self, path: &Path, autoplay: bool, ranges: Option<Vec<(i64, i64)>>) -> Result<OpenInfo, PlayerError> {
        self.host.open(OpenRequest { asset_id: 1, path: path.to_path_buf(), autoplay, canvas: (1280, 720), ranges, duration_hint_ms: None, start_ms: None })
    }

    fn last(&self) -> Option<PlayerEvent> {
        self.events.lock().unwrap().last().cloned()
    }

    fn wait_event(&self, ms: u64, pred: impl Fn(&PlayerEvent) -> bool) -> Option<PlayerEvent> {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            if let Some(e) = self.events.lock().unwrap().iter().rev().find(|e| pred(e)).cloned() {
                return Some(e);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// `act` 를 실행한 뒤 새로 나온 이벤트 중 시각이 `ms` 에 ±`tol` 인 것을 기다린다 (이전 이벤트는 보지 않는다)
    fn after(&self, act: impl FnOnce(), ms: i64, tol: i64) -> bool {
        let mark = self.events.lock().unwrap().len();
        act();
        wait(4000, || self.events.lock().unwrap().iter().skip(mark).any(|e| (e.time_ms - ms).abs() <= tol))
    }

    fn frames(&self) -> usize {
        self.surface.frame_count()
    }
}

/// 프레임 수가 300ms 동안 변하지 않을 때까지 기다린다 (상한 5초)
fn settle(l: &Live) {
    let end = Instant::now() + Duration::from_secs(5);
    let (mut last, mut since) = (l.frames(), Instant::now());
    while Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
        if l.frames() != last {
            last = l.frames();
            since = Instant::now();
        } else if since.elapsed() >= Duration::from_millis(300) {
            return;
        }
    }
}

fn wait(ms: u64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f()
}

#[test]
fn t_player_live_01_open_paused_shows_the_first_frame_without_playing() {
    let Some(l) = live("t_player_live_01") else { return };
    let p = l.copy("h264_1080p30.mp4");
    let info = l.open(&p, false, None).expect("open");
    assert!(info.paused && info.time_ms == 0, "{info:?}");
    assert!(info.length_ms.is_some_and(|v| (29_900..=30_100).contains(&v)), "{info:?}");
    assert!(info.fps.is_some_and(|f| (f - 30.0).abs() < 0.01));
    // 실제 프레임이 게시됐다 (검은 화면이 아니다): 새 epoch 슬롯의 픽셀에 밝은 값이 있다
    assert!(wait(5000, || l.surface.last_frame().is_some()), "no frame");
    let f = l.surface.last_frame().expect("frame");
    assert_eq!((f.w, f.h), (1280, 720));
    let px = l.surface.read_slot(f.epoch, f.slot as usize).expect("slot");
    assert!(px.iter().step_by(4).any(|&b| b > 100), "프레임이 비어 있다");
    // 정지로 열었으니 움직이지 않는다 (재생 이벤트·프레임 증가 없음)
    std::thread::sleep(Duration::from_millis(300));
    let n = l.frames();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(l.frames(), n, "정지로 열면 프레임이 흐르지 않는다");
    assert!(!l.events.lock().unwrap().iter().any(|e| e.state == EventState::Playing));
    l.host.shutdown(Duration::from_secs(3));
}

#[test]
fn t_player_live_02_seek_step_forward_and_back_report_exact_frame_times() {
    let Some(l) = live("t_player_live_02") else { return };
    let p = l.copy("h264_1080p30.mp4");
    l.open(&p, false, None).expect("open");
    // seek 는 상태를 바꾸지 않고 프레임 시작 시각에 정확히 착지한다 (UI 가 보내는 ms 반올림 값 → 가까운 프레임)
    assert!(l.after(|| l.host.seek(1000).unwrap(), 1000, 1), "{:?}", l.last());
    assert_eq!(l.last().unwrap().state, EventState::Paused);
    assert!(l.after(|| l.host.control(ControlAction::StepForward).unwrap(), 1033, 1), "{:?}", l.last());
    assert!(l.after(|| l.host.control(ControlAction::StepBack).unwrap(), 1000, 1), "{:?}", l.last());
    assert!(l.after(|| l.host.control(ControlAction::StepBack).unwrap(), 967, 1), "{:?}", l.last());
    // 첫 프레임에서 뒤로: 아무 일도 하지 않지만 완료를 알린다
    assert!(l.after(|| l.host.seek(0).unwrap(), 0, 0));
    // (같은 상태·시각이라 호스트가 중복 이벤트를 걸러 새 이벤트는 없다 — 워커가 멈추지 않고 시각이 0 으로 유지되는지 본다)
    l.host.control(ControlAction::StepBack).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(l.last().unwrap().time_ms, 0);
    assert!(l.after(|| l.host.seek(500).unwrap(), 500, 1), "no-op 이동 뒤에도 워커가 응답한다");
    l.host.shutdown(Duration::from_secs(3));
}

/// ffmpeg 로 첫 프레임 pts 가 0.105s 인 시료를 만든다 (없으면 None)
fn offset_sample(dir: &Path, src: &Path) -> Option<PathBuf> {
    let out = dir.join("offset.mkv");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-t", "4", "-an", "-c:v", "copy", "-output_ts_offset", "0.105", "-f", "matroska"])
        .arg(&out)
        .status()
        .ok()?
        .success();
    ok.then_some(out)
}

#[test]
fn t_player_live_03_non_zero_start_offset_times_are_relative_and_seeks_land_on_the_grid() {
    let Some(l) = live("t_player_live_03") else { return };
    let src = l.copy("h264_1080p30.mp4");
    let Some(p) = offset_sample(l.media.path(), &src) else {
        println!("SKIP t_player_live_03: ffmpeg is not available to build the offset sample");
        return;
    };
    // 첫 프레임 pts 0.105 인 파일 — 호스트가 보는 시각은 첫 프레임 = 0
    let info = l.open(&p, false, None).expect("open");
    assert_eq!(info.time_ms, 0, "{info:?}");
    for ms in [500, 1000, 2033, 133] {
        assert!(l.after(|| l.host.seek(ms).unwrap(), ms, 1), "seek {ms}: {:?}", l.last());
    }
    // 프레임 이동도 격자 위 — 33ms 간격으로 이어진다 (0 기준 격자였다면 한 칸씩 +21.8ms 어긋났다)
    assert!(l.after(|| l.host.seek(1000).unwrap(), 1000, 1));
    assert!(l.after(|| l.host.control(ControlAction::StepForward).unwrap(), 1033, 1), "{:?}", l.last());
    l.host.shutdown(Duration::from_secs(3));
}

#[test]
fn t_player_live_04_ranges_play_as_edl_with_source_times_and_survive_set_ranges() {
    let Some(l) = live("t_player_live_04") else { return };
    let p = l.copy("h264_1080p30.mp4");
    // Gate ON: 구간 사용 — 소스 시각 기준 첫 구간 시작에서 정지로 열린다
    let info = l.open(&p, false, Some(vec![(1000, 2000), (5000, 6000)])).expect("open");
    assert_eq!(info.time_ms, 1000, "{info:?}");
    assert!(info.length_ms.is_some_and(|v| (29_900..=30_100).contains(&v)), "길이는 소스 전체: {info:?}");
    // 구간 안 seek 는 소스 시각 그대로, 구간 사이는 다음 구간 시작
    assert!(l.after(|| l.host.seek(5500).unwrap(), 5500, 1), "{:?}", l.last());
    assert!(l.after(|| l.host.seek(3000).unwrap(), 5000, 1), "{:?}", l.last());
    // 마지막 구간 끝까지 재생하면 끝(ended)
    assert!(l.after(|| l.host.seek(5900).unwrap(), 5900, 1));
    l.host.set_rate(4.0).unwrap();
    l.host.control(ControlAction::Play).unwrap();
    let ended = l.wait_event(6000, |e| e.state == EventState::Ended).expect("ended");
    assert!((5900..=6000).contains(&ended.time_ms), "{ended:?}");
    // 열린 채 구간 교체: 위치를 유지한 채 구간 해제 → 같은 소스 시각, 정지 유지
    l.host.set_rate(1.0).unwrap();
    assert!(l.after(|| l.host.seek(5500).unwrap(), 5500, 1));
    l.host.set_ranges(1, None).unwrap();
    assert!(l.wait_event(5000, |e| e.state == EventState::Paused && (e.time_ms - 5500).abs() <= 1 && e.time_ms == 5500).is_some(), "{:?}", l.last());
    // 일반 재생이 됐으므로 구간 밖(0)으로도 이동할 수 있다
    assert!(l.after(|| l.host.seek(100).unwrap(), 100, 1), "{:?}", l.last());
    l.host.shutdown(Duration::from_secs(3));
}

#[test]
fn t_player_live_05_killed_worker_is_restarted_and_the_position_is_restored() {
    let Some(l) = live("t_player_live_05") else { return };
    let p = l.copy("h264_1080p30.mp4");
    l.open(&p, false, None).expect("open");
    assert!(l.after(|| l.host.seek(2000).unwrap(), 2000, 1));
    let epoch_before = l.host.current_epoch();
    let frames_before = l.frames();
    let mark = l.events.lock().unwrap().len();
    let t0 = Instant::now();
    // Act: 워커 강제 종료 (TerminateProcess)
    l.host.kill_worker_for_test();
    // Assert: restarting → 같은 위치로 복원 (정지 유지) + 새 epoch 로 프레임 재게시
    assert!(l.wait_event(8000, |e| e.state == EventState::Restarting).is_some());
    assert!(wait(10_000, || l.host.current_epoch() > epoch_before && l.frames() > frames_before), "frames after restart");
    let restored = wait(10_000, || l.events.lock().unwrap().iter().skip(mark).any(|e| e.state == EventState::Paused && e.time_ms == 2000));
    assert!(restored, "{:?}", l.events.lock().unwrap().iter().skip(mark).map(|e| (e.state, e.time_ms)).collect::<Vec<_>>());
    println!("restore took {:?}", t0.elapsed());
    assert!(t0.elapsed() < Duration::from_secs(8));
    // 복원 뒤에도 명령이 동작한다
    assert!(l.after(|| l.host.control(ControlAction::StepForward).unwrap(), 2033, 1), "{:?}", l.last());
    l.host.shutdown(Duration::from_secs(3));
}

#[test]
fn t_player_live_06_render_inactive_stops_frames_and_resume_publishes_the_current_frame() {
    let Some(l) = live("t_player_live_06") else { return };
    let p = l.copy("h264_1080p30.mp4");
    l.open(&p, false, None).expect("open");
    // 열기 직후의 프레임이 모두 도착해 잠잠해질 때까지 기다린다
    settle(&l);
    // Gate ON: 렌더 꺼짐 — seek 해도 프레임을 게시하지 않는다
    l.host.set_render_active(false);
    l.host.seek(0).unwrap(); // 큐 순서상 위 요청이 먼저 처리된다
    std::thread::sleep(Duration::from_millis(300));
    let n = l.frames();
    assert!(l.after(|| l.host.seek(3000).unwrap(), 3000, 1), "시각은 계속 갱신된다");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(l.frames(), n, "렌더가 꺼져 있으면 프레임이 나가지 않는다");
    // 켜면 현재 프레임을 한 번 다시 게시한다
    l.host.set_render_active(true);
    assert!(wait(3000, || l.frames() > n), "렌더 재개 시 현재 프레임 게시");
    // 이미 켜진 상태에서 다시 켜도(WebGL 컨텍스트 복구·접힌 스테이지 복귀) 정지 상태의 현재 프레임을 다시 게시한다
    std::thread::sleep(Duration::from_millis(300));
    let m = l.frames();
    l.host.set_render_active(true);
    assert!(wait(3000, || l.frames() > m), "true→true 도 현재 프레임을 다시 게시: {:?}", l.last());
    l.host.shutdown(Duration::from_secs(3));
}

fn tree(dir: &Path) -> BTreeMap<PathBuf, (u64, u128)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let (p, Ok(md)) = (e.path(), e.metadata()) else { continue };
            if md.is_dir() {
                stack.push(p);
            } else {
                let m = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
                out.insert(p, (md.len(), m));
            }
        }
    }
    out
}

fn fingerprint(p: &Path) -> (u64, u128, u64) {
    use std::hash::{Hash, Hasher};
    let bytes = std::fs::read(p).unwrap();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    let md = std::fs::metadata(p).unwrap();
    (md.len(), md.modified().unwrap().duration_since(UNIX_EPOCH).unwrap().as_nanos(), h.finish())
}

#[test]
fn t_player_orig_01_playback_never_modifies_the_original_or_writes_outside_the_data_dir() {
    let Some(l) = live("t_player_orig_01") else { return };
    let p = l.copy("h264_1080p30.mp4");
    let sub = l.data.path().join("cache").join("player-subtitles");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::copy(l.samples.join("test_sub.srt"), sub.join("sub-0.srt")).ok();
    // 사용자 프로필의 mpv 설정·캐시 폴더 — 워커가 만들면 안 된다
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    let watch: Vec<PathBuf> = [local.map(|d| d.join("mpv")), roaming.map(|d| d.join("mpv")), Some(std::env::current_dir().unwrap())].into_iter().flatten().collect();
    let watch_before: Vec<_> = watch.iter().map(|d| tree(d)).collect();
    let before = fingerprint(&p);
    let media_before = tree(l.media.path());

    // Act: 재생·seek·프레임 이동·구간·자막·음량·배속·닫기까지 쓰기 가능한 경로를 모두 지난다
    l.open(&p, true, None).expect("open");
    std::thread::sleep(Duration::from_millis(800));
    l.host.seek(2000).unwrap();
    l.host.control(ControlAction::StepForward).unwrap();
    l.host.control(ControlAction::StepBack).unwrap();
    l.host.set_ranges(1, Some(vec![(1000, 3000)])).unwrap();
    let srt = sub.join("sub-0.srt");
    if srt.is_file() {
        l.host.set_subtitle(1, Some(srt)).unwrap();
        l.host.set_subtitle(1, None).unwrap();
    }
    l.host.set_volume(30, false).unwrap();
    l.host.set_rate(2.0).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    l.host.close().unwrap();
    l.host.shutdown(Duration::from_secs(3));

    // Assert: 내용·수정 시각 불변, 원본 폴더에 부산물 없음
    assert_eq!(fingerprint(&p), before);
    assert_eq!(tree(l.media.path()), media_before, "원본 폴더에 파일이 생기거나 바뀌었다");
    // 사용자 프로필·작업 폴더에도 새 파일이 생기지 않았다
    for (d, b) in watch.iter().zip(watch_before) {
        assert_eq!(&tree(d), &b, "{} 에 파일이 생겼다", d.display());
    }
}

#[test]
fn t_player_orig_02_opening_a_missing_file_fails_cleanly_and_creates_nothing() {
    let Some(l) = live("t_player_orig_02") else { return };
    let missing = l.media.path().join("nope.mp4");
    let e = l.open(&missing, true, None).unwrap_err();
    assert!(matches!(e, PlayerError::OpenFailed(_)), "{e}");
    assert!(tree(l.media.path()).is_empty(), "없는 파일을 열려다 파일이 생겼다");
    // 실패 뒤에도 정상 파일을 열 수 있다
    let p = l.copy("h264_1080p30.mp4");
    assert!(l.open(&p, false, None).is_ok());
    l.host.shutdown(Duration::from_secs(3));
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

// ---- 워커와 직접 줄 JSON 으로 말한다 (호스트 없이): mpv 옵션이 실제로 적용됐는지 Probe 로 확인 ----

use mam_player::host::{ProcessSpawner, WorkerSpawner};
use mam_player::protocol::{Command, Message, Request, decode_line, encode_line};
use std::io::{BufRead, BufReader, Write};

struct Raw {
    cmd: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Message>,
    next: u64,
    _proc: Box<dyn mam_player::host::WorkerProcess>,
}

impl Raw {
    fn start(mpv: PathBuf) -> Raw {
        let s = ProcessSpawner { worker_exe: PathBuf::from(WORKER), libmpv_dir: mpv, inherit_stderr: false, env: vec![("MAM_PLAYER_AO".into(), "null".into())] };
        let conn = s.spawn().expect("spawn");
        let (tx, rx) = std::sync::mpsc::channel();
        let evt = conn.evt;
        std::thread::spawn(move || {
            for line in BufReader::new(evt).lines().map_while(Result::ok) {
                if let Ok(m) = decode_line::<Message>(&line) {
                    let _ = tx.send(m);
                }
            }
        });
        Raw { cmd: conn.cmd, rx, next: 0, _proc: conn.process }
    }

    fn send(&mut self, command: Command) -> u64 {
        self.next += 1;
        let line = encode_line(&Request { id: self.next, command }).unwrap();
        self.cmd.write_all(line.as_bytes()).unwrap();
        self.next
    }

    fn until(&self, ms: u64, pred: impl Fn(&Message) -> bool) -> Option<Message> {
        let end = Instant::now() + Duration::from_millis(ms);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            match self.rx.recv_timeout(left) {
                Ok(m) if pred(&m) => return Some(m),
                Ok(_) => {}
                Err(_) => return None,
            }
        }
        None
    }

    fn reply(&mut self, c: Command) -> Message {
        let id = self.send(c);
        self.until(8000, |m| matches!(m, Message::Reply { id: i, .. } if *i == id)).expect("reply")
    }
}

#[test]
fn t_player_live_07_raw_worker_applies_protection_defaults_volume_curve_and_rejects_bad_input() {
    let (Some(mpv), Some(samples)) = (libmpv_dir(), samples_dir()) else {
        println!("SKIP t_player_live_07: libmpv or samples not available");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.mp4");
    std::fs::copy(samples.join("h264_1080p30.mp4"), &p).unwrap();
    let mut w = Raw::start(mpv);
    // Ready: 프로토콜 버전·렌더 경로
    let ready = w.until(5000, |m| matches!(m, Message::Ready { .. })).expect("ready");
    assert!(matches!(ready, Message::Ready { protocol, .. } if protocol == mam_player::protocol::PROTOCOL_VERSION));
    // 열기 전: 세션이 필요한 명령은 NoMedia, 잘못된 인자는 InvalidArgument
    assert!(matches!(w.reply(Command::Seek { ms: 5 }), Message::Reply { ok: false, code: Some(mam_player::protocol::WorkerErrorCode::NoMedia), .. }));
    assert!(matches!(w.reply(Command::SetSpeed { speed: 99.0 }), Message::Reply { ok: false, code: Some(mam_player::protocol::WorkerErrorCode::InvalidArgument), .. }));
    assert!(matches!(w.reply(Command::Open { path: String::new(), paused: true, ranges: None, duration_ms: None, start_ms: None }), Message::Reply { ok: false, .. }));
    assert!(matches!(w.reply(Command::Open { path: "x".into(), paused: true, ranges: Some(vec![(5, 5)]), duration_ms: None, start_ms: None }), Message::Reply { ok: false, .. }), "잘못된 구간");
    assert!(matches!(w.reply(Command::Attach { epoch: 1, w: 0, h: 0, stride: 0, size: 0, handles: vec![1, 2, 3] }), Message::Reply { ok: false, .. }), "잘못된 버퍼 크기");
    // 정상 열기 (버퍼 없이도 열린다 — 프레임은 소비만 한다)
    assert!(matches!(w.reply(Command::Open { path: p.to_string_lossy().into_owned(), paused: true, ranges: None, duration_ms: None, start_ms: None }), Message::Reply { ok: true, .. }));
    w.until(8000, |m| matches!(m, Message::PlaybackRestart { .. })).expect("restart");
    assert!(matches!(w.reply(Command::SetVolume { volume: 30, mute: false }), Message::Reply { ok: true, .. }));
    assert!(matches!(w.reply(Command::SetSpeed { speed: 1.5 }), Message::Reply { ok: true, .. }));
    let Message::Reply { data: Some(d), .. } = w.reply(Command::Probe) else { panic!("probe") };
    // 음량 곡선: mpv = 100·∛(0.3) ≈ 66.9 (진폭 = 0.3)
    assert!(d.volume.is_some_and(|v| (v - 66.94).abs() < 0.1), "{d:?}");
    assert_eq!(d.speed, Some(1.5));
    assert_eq!(d.audio_delay, Some(0.011), "audio-delay 보정 기본값");
    assert_eq!(d.paused, Some(true));
    assert!(!d.edl);
    assert_eq!(d.video_format.as_deref(), Some("h264"));
    // 음량 0 또는 음소거는 무음
    assert!(matches!(w.reply(Command::SetVolume { volume: 0, mute: false }), Message::Reply { ok: true, .. }));
    let Message::Reply { data: Some(d), .. } = w.reply(Command::Probe) else { panic!("probe") };
    assert_eq!(d.mute, Some(true));
    let _ = w.reply(Command::Quit);
}

/// 요청한 렌더 크기와 실제 게시된 프레임 크기가 같은지 (상한 아래면 정확히 같고, 위면 종횡비를 지키며 줄어든다)
fn check_published_sizes(name: &str, env: Vec<(String, String)>) {
    let Some(l) = live_env(name, env) else { return };
    let p = l.copy("smpte_1080p30.mp4");
    l.open(&p, false, None).expect("open");
    // (요청, 기대) — 홀수 너비 1333, 상한 초과(가로·세로·둘 다), 극단 종횡비
    let cases = [
        ((1333, 750), (1333, 750)),
        ((1001, 563), (1001, 563)),
        ((641, 361), (641, 361)),
        ((1305, 750), (1305, 750)),
        ((1920, 1080), (1920, 1080)),
        ((3000, 1500), (1920, 960)),
        ((2561, 1441), (1919, 1080)),
        ((333, 750), (333, 750)),
    ];
    for ((rw, rh), (ew, eh)) in cases {
        let before = l.host.current_epoch();
        l.host.set_render_size(rw, rh).unwrap();
        assert!(wait(5000, || l.host.current_epoch() > before && l.surface.last_frame().is_some_and(|f| f.epoch == l.host.current_epoch())), "{rw}x{rh}: no frame in the new epoch");
        let f = l.surface.last_frame().unwrap();
        assert_eq!((f.w, f.h), (ew, eh), "요청 {rw}x{rh}");
        assert!(f.stride >= f.w * 4 && f.stride % 64 == 0, "{f:?}");
        // 상한 초과는 종횡비를 1px 이내로 지킨다
        let (ra, ea) = (rw as f64 / rh as f64, f.w as f64 / f.h as f64);
        assert!((ra - ea).abs() * f.h as f64 <= 1.0, "{rw}x{rh} -> {}x{}", f.w, f.h);
        // 가장자리 열이 비어(검은 띠) 있지 않다 — SMPTE 막대의 양끝은 밝은 회색(왼쪽)·파랑(오른쪽)이다. 16:9 영상의 1333x750 에서 1px 미만 반올림이 띠로 보이면 안 된다
        if (rw, rh) == (1333, 750) {
            let px = l.surface.read_slot(f.epoch, f.slot as usize).unwrap();
            let at = |x: usize, y: usize| { let o = y * f.stride as usize + x * 4; (px[o] as u32 + px[o + 1] as u32 + px[o + 2] as u32) / 3 };
            let y = f.h as usize / 4;
            assert!(at(0, y) > 60, "왼쪽 가장자리 열이 어둡다: {}", at(0, y));
            assert!(at(f.w as usize - 1, y) > 30, "오른쪽 가장자리 열이 어둡다: {}", at(f.w as usize - 1, y));
        }
        // 슬롯 메모리가 stride*h 만큼 실제로 있다
        assert!(l.surface.read_slot(f.epoch, f.slot as usize).is_some_and(|b| b.len() as u64 >= f.stride as u64 * f.h as u64));
        std::thread::sleep(Duration::from_millis(300)); // 250ms 교체 간격
    }
    l.host.shutdown(Duration::from_secs(3));
}

#[test]
fn t_player_size_01_requested_render_size_equals_the_published_frame_size_r2() {
    check_published_sizes("t_player_size_01", vec![("MAM_PLAYER_RENDER_API".into(), "d3d11".into())]);
}

#[test]
fn t_player_size_02_requested_render_size_equals_the_published_frame_size_r1() {
    check_published_sizes("t_player_size_02", vec![("MAM_PLAYER_RENDER_API".into(), "sw".into())]);
}

// 분류: Happy (이어보기 시작 위치) / Boundary (끝 근처는 처음부터)
// 실행 환경(libmpv·시료)이 없으면 건너뛴다 - 이 시험은 libmpv 가 있는 PC 에서만 실제로 돈다
#[test]
fn t_resume_live_01_open_with_start_ms_lands_on_the_position_without_showing_frame_zero() {
    let Some(l) = live("t_resume_live_01") else { return };
    let p = l.copy("h264_1080p30.mp4");
    let info = l
        .host
        .open(OpenRequest { asset_id: 1, path: p.clone(), autoplay: false, canvas: (1280, 720), ranges: None, duration_hint_ms: None, start_ms: Some(10_000) })
        .expect("open");
    assert!(info.paused && (info.time_ms - 10_000).abs() <= 40, "{info:?}");
    // 0 초 프레임이 열기 도중 보인 적이 없다 (게시된 프레임이 있어도 이벤트 시각은 시작 위치 이후)
    assert!(l.events.lock().unwrap().iter().all(|e| e.time_ms == 0 || e.time_ms >= 9_900), "{:?}", l.events.lock().unwrap());
    // 끝 근처(길이 0.5초 이내)는 이동하지 않고 처음에서 열린다 (EOF 로 열기가 멈추지 않는다)
    let near_end = l
        .host
        .open(OpenRequest { asset_id: 1, path: p, autoplay: false, canvas: (1280, 720), ranges: None, duration_hint_ms: None, start_ms: Some(29_900) })
        .expect("open near end");
    assert!(near_end.time_ms <= 100, "{near_end:?}");
    l.host.shutdown(Duration::from_secs(3));
}
