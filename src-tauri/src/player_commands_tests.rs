// Categories: Validation (계약 인자·오류 코드·golden) / Business rule (오프라인 -> asset_offline, 엔진 없음 폴백, 렌더 켜짐 게이트) /
// External (워커 없음·크래시) / 원본 보호 (내용·수정 시각 불변)
// 실제 SQLite + 실제 스캔 + 가짜 워커(`mam_player::host::testing`) — 실제 libmpv 시험은 `mam-player-worker/tests/live.rs` 가 맡는다.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::reuse::{ShellLauncher, StdFs};
use mam_core::scan::scan_folder;
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, Priority, SystemClock, migrate, open};
use mam_player::host::testing::{FakeScript, FakeSpawner, MemSurface};
use mam_player::host::{EventState, PlayerErrorCode, Timeouts};
use mam_player::protocol::Command;

use super::*;

type Events = Arc<Mutex<Vec<PlayerEvent>>>;

fn wav_bytes() -> Vec<u8> {
    let rate: u32 = 8000;
    let samples: Vec<i16> = (0..rate * 3).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16).collect();
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

pub(crate) fn canvas() -> CanvasSize {
    CanvasSize { width: 640, height: 360 }
}

pub(crate) struct Fx {
    _dir: tempfile::TempDir,
    pub(crate) data: PathBuf,
    pub(crate) media: PathBuf,
    pool: Arc<DbPool>,
    pub(crate) reuse: Arc<Reuse>,
    pub(crate) wav_id: i64,
    _ingest: Arc<Ingest>,
}

/// 실제 SQLite + 실제 스캔: `media/clip.wav` 한 건
pub(crate) fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("MAMData");
    let media = base.join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(media.join("clip.wav"), wav_bytes()).unwrap();
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &data.join("backups"), &SystemClock).unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&pool),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig { auto_excludes: vec![data.clone()], ..IngestConfig::default() },
    );
    let view = ingest.add_folder(media.to_str().unwrap()).unwrap();
    scan_folder(ingest.env(), view.folder_id, &|| false, &AtomicBool::new(false)).unwrap();
    let wav_id = pool
        .read(|c| Ok(c.query_row("SELECT asset_id FROM file_locations WHERE rel_path LIKE '%clip.wav'", [], |r| r.get::<_, i64>(0))))
        .unwrap()
        .unwrap();
    let reuse = Arc::new(Reuse::new(Arc::clone(&pool), Arc::new(SystemClock), Arc::new(StdFs), Arc::new(ShellLauncher), vec![data.clone()]));
    Fx { _dir: dir, data, media, pool, reuse, wav_id, _ingest: ingest }
}

pub(crate) fn set_volume_offline(fx: &Fx) {
    fx.pool
        .write(Priority::High, |c| {
            c.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();
}

fn fast() -> Timeouts {
    Timeouts {
        open: Duration::from_millis(1500),
        command: Duration::from_millis(250),
        silence: Duration::from_secs(3),
        backoff: [Duration::ZERO, Duration::from_millis(10), Duration::from_millis(20)],
        frame_grace: Duration::from_millis(100),
        ..Timeouts::default()
    }
}

/// 가짜 워커로 동작하는 서비스 (실제 프로세스·libmpv 없음)
pub(crate) struct Eng {
    pub(crate) svc: PlayerService,
    pub(crate) events: Events,
    pub(crate) script: Arc<FakeScript>,
}

pub(crate) fn engine(fx: &Fx) -> Eng {
    let script = FakeScript::new();
    let events: Events = Arc::default();
    let ev = Arc::clone(&events);
    let host = PlayerHost::with_spawner(
        HostConfig { timeouts: fast(), ..HostConfig::default() },
        MemSurface::new(),
        Arc::new(move |e| ev.lock().unwrap().push(e)),
        Some(FakeSpawner::new(Arc::clone(&script))),
    );
    Eng { svc: PlayerService::with_host(Arc::new(host), fx.data.join("cache")), events, script }
}

/// 워커·libmpv 가 없는 설치 (`player_unavailable`)
pub(crate) fn no_engine(fx: &Fx) -> (PlayerService, Events) {
    let events: Events = Arc::default();
    let ev = Arc::clone(&events);
    let host = PlayerHost::with_spawner(HostConfig::default(), MemSurface::new(), Arc::new(move |e| ev.lock().unwrap().push(e)), None);
    (PlayerService::with_host(Arc::new(host), fx.data.join("cache")), events)
}

pub(crate) fn code_of(e: &CommandError) -> serde_json::Value {
    serde_json::to_value(e).unwrap()["code"].clone()
}

pub(crate) fn fingerprint(p: &Path) -> (Vec<u8>, std::time::SystemTime) {
    (std::fs::read(p).unwrap(), std::fs::metadata(p).unwrap().modified().unwrap())
}

fn wait_for(events: &Events, ms: u64, pred: impl Fn(&PlayerEvent) -> bool) -> Option<PlayerEvent> {
    let end = Instant::now() + Duration::from_millis(ms);
    loop {
        if let Some(e) = events.lock().unwrap().iter().find(|e| pred(e)) {
            return Some(e.clone());
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn range(s: i64, e: i64) -> PlayRange {
    serde_json::from_value(serde_json::json!({"startMs": s, "endMs": e})).unwrap()
}

fn fixture_json() -> serde_json::Value {
    let raw = include_str!("../../src/test/fixtures/player-wire.json").replace("\r\n", "\n");
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn t_ipc_player_mpv_01_argument_and_reply_shapes_match_the_golden_contract() {
    // Arrange / Act: 프론트가 보내는 camelCase JSON
    let c: CanvasSize = serde_json::from_value(serde_json::json!({"width": 1280, "height": 720})).unwrap();
    let a: ControlAction = serde_json::from_value(serde_json::json!("stepBack")).unwrap();
    let fx = fixture_json();
    let reply = serde_json::to_value(OpenReply { session_id: 5, length_ms: Some(120_000), fps: Some(29.97002997002997), state: ReplyState::Paused, time_ms: 0 }).unwrap();
    // Assert
    assert_eq!((c.width, c.height), (1280, 720));
    assert_eq!(a, ControlAction::StepBack);
    assert_eq!(reply, fx["openReply"], "프론트(vitest)와 공유하는 golden fixture");
    assert_eq!(serde_json::to_value(OpenReply { session_id: 1, length_ms: None, fps: None, state: ReplyState::Playing, time_ms: 7 }).unwrap(), serde_json::json!({"sessionId": 1, "lengthMs": null, "fps": null, "state": "playing", "timeMs": 7}));
    assert_eq!(PLAYER_STATE_EVENT, "player://state");
    // 누락·잘못된 필드는 거부 (구 계약의 rect·fps 는 더 이상 받지 않는다)
    assert!(serde_json::from_value::<CanvasSize>(serde_json::json!({"width": 1})).is_err());
    assert!(serde_json::from_value::<CanvasSize>(serde_json::json!({"width": -1, "height": 2})).is_err());
    assert!(serde_json::from_value::<ControlAction>(serde_json::json!("fastForward")).is_err());
}

#[test]
fn t_ipc_player_mpv_02_engine_errors_map_to_contract_codes_without_paths() {
    // Arrange
    let secret = r"C:\Users\someone\Videos\secret name.mxf".to_string();
    let unavailable = vec![
        PlayerError::WorkerNotFound("worker"),
        PlayerError::SpawnFailed(secret.clone()),
        PlayerError::ProtocolMismatch { worker: 9, host: 1 },
        PlayerError::SurfaceUnavailable(secret.clone()),
    ];
    // Act / Assert: 열기
    for e in unavailable {
        let mapped = open_error(e);
        assert_eq!(code_of(&mapped), "player_unavailable");
        assert!(!serde_json::to_string(&mapped).unwrap().contains("Users"));
    }
    for (e, code) in [
        (PlayerError::OpenFailed(secret.clone()), "open_failed"),
        (PlayerError::Cancelled, "open_failed"),
        (PlayerError::HostGone, "open_failed"),
        (PlayerError::Timeout, "player_timeout"),
        (PlayerError::RestartFailed, "player_restart_failed"),
    ] {
        let mapped = open_error(e);
        assert_eq!(code_of(&mapped), code);
        assert!(!serde_json::to_string(&mapped).unwrap().contains("Users"));
    }
    // 일반 command
    assert_eq!(code_of(&command_error(PlayerError::NoMedia)), "not_found");
    assert_eq!(code_of(&command_error(PlayerError::Timeout)), "player_timeout");
    assert_eq!(code_of(&command_error(PlayerError::Restarting)), "player_restarting");
    assert_eq!(code_of(&command_error(PlayerError::RestartFailed)), "player_restart_failed");
    assert_eq!(code_of(&command_error(PlayerError::InvalidArgument(secret))), "invalid_input");
    assert_eq!(code_of(&command_error(PlayerError::SpawnFailed("x".into()))), "player_unavailable");
    assert_eq!(code_of(&command_error(PlayerError::HostGone)), "internal");
}

#[test]
fn t_ipc_player_mpv_03_event_error_codes_use_the_same_strings_as_the_command_error_codes() {
    for (a, b) in [(PlayerErrorCode::OpenFailed, ErrorCode::OpenFailed), (PlayerErrorCode::PlayerWorkerCrashed, ErrorCode::PlayerWorkerCrashed)] {
        assert_eq!(serde_json::to_value(a).unwrap(), serde_json::to_value(b).unwrap());
    }
}

#[test]
fn t_ipc_player_mpv_04_unavailable_engine_and_offline_assets() {
    // Arrange: 워커·libmpv 없음
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    // Act / Assert: 가용성 false, 열기는 player_unavailable (프론트는 외부 열기로 폴백)
    assert!(!svc.available());
    for autoplay in [true, false] {
        assert_eq!(code_of(&svc.open(&fx.reuse, fx.wav_id, canvas(), autoplay, None, None, None).unwrap_err()), "player_unavailable", "{autoplay}");
        assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, canvas(), autoplay, None, None, None).unwrap_err()), "asset_offline", "{autoplay}: 자산 해석이 먼저");
    }
    // 열기 전 제어 명령은 오류 없이 무시된다
    svc.control(ControlAction::Toggle).unwrap();
    svc.seek(5.0).unwrap();
    svc.set_render_size(100, 100).unwrap();
    svc.close().unwrap();
    assert!(events.lock().unwrap().is_empty());
    // 파일이 사라진 자산 (볼륨은 연결됨) -> asset_offline
    let (original, backup) = (fx.media.join("clip.wav"), fx.media.join("clip.bak"));
    std::fs::rename(&original, &backup).unwrap();
    assert_eq!(code_of(&svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).unwrap_err()), "asset_offline");
    std::fs::rename(&backup, &original).unwrap();
    // 볼륨 오프라인 -> asset_offline, 경로를 싣지 않는다
    set_volume_offline(&fx);
    let err = svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).unwrap_err();
    assert_eq!(code_of(&err), "asset_offline");
    assert!(!serde_json::to_string(&err).unwrap().contains("clip.wav"));
}

#[test]
fn t_ipc_player_mpv_05_invalid_arguments_are_rejected_before_the_asset_lookup_and_the_engine() {
    let fx = fixture();
    let e = engine(&fx);
    let svc = &e.svc;
    for bad in [f64::NAN, f64::INFINITY] {
        assert_eq!(code_of(&svc.seek(bad).unwrap_err()), "invalid_input");
        assert_eq!(code_of(&svc.set_volume(bad, false).unwrap_err()), "invalid_input");
    }
    for bad in [0.0, -1.0, 0.05, 17.0, f64::NAN] {
        assert_eq!(code_of(&svc.set_rate(bad).unwrap_err()), "invalid_input", "{bad}");
    }
    // 캔버스·렌더 크기: 0 과 16385 는 거부 (존재하지 않는 자산이어도 invalid_input 이 먼저), 경계 1·16384 는 통과
    for (w, h) in [(0, 10), (10, 0), (16385, 10), (10, 16385), (0, 0)] {
        assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, CanvasSize { width: w, height: h }, true, None, None, None).unwrap_err()), "invalid_input", "{w}x{h}");
        assert_eq!(code_of(&svc.set_render_size(w, h).unwrap_err()), "invalid_input", "{w}x{h}");
    }
    assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, CanvasSize { width: 1, height: 16384 }, true, None, None, None).unwrap_err()), "asset_offline");
    svc.set_render_size(1, 16384).unwrap();
    // 정상 범위 (열린 미디어가 없으므로 무시) - 음수 seek·범위 밖 음량은 보정
    svc.seek(-5.0).unwrap();
    svc.set_rate(1.5).unwrap();
    svc.set_volume(250.0, true).unwrap();
    assert_eq!(e.script.spawns.load(Ordering::SeqCst), 0, "세션 없는 호출은 워커를 띄우지 않는다");
}

#[test]
fn t_ipc_player_mpv_06_open_play_close_emit_state_and_leave_original_untouched() {
    // Arrange
    let fx = fixture();
    let original = fx.media.join("clip.wav");
    let before = fingerprint(&original);
    let e = engine(&fx);
    assert!(e.svc.available());
    // Act
    let reply = e.svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, Some(3000), None).expect("open");
    e.svc.control(ControlAction::Pause).unwrap();
    e.svc.seek(1000.0).unwrap();
    e.svc.set_volume(40.0, false).unwrap();
    let paused = wait_for(&e.events, 2000, |ev| ev.state == EventState::Paused && ev.time_ms == 1000);
    e.svc.close().unwrap();
    e.svc.shutdown();
    // Assert: 응답이 첫 상태를 준다 (열기 뒤 첫 프레임 이벤트는 없다) · 이벤트 모양에 경로가 없다
    assert_eq!((reply.session_id, reply.state, reply.time_ms, reply.length_ms), (1, ReplyState::Playing, 0, Some(30_000)));
    let json = serde_json::to_value(paused.expect("paused event")).unwrap();
    assert_eq!((json["assetId"].as_i64(), json["sessionId"].as_u64()), (Some(fx.wav_id), Some(1)));
    assert!(!json.to_string().contains("clip.wav"));
    assert_eq!(e.events.lock().unwrap()[0].state, EventState::Opening);
    // 힌트(자산 행의 소스 길이)가 워커의 Open 으로 간다
    assert!(e.script.commands().iter().any(|c| matches!(c, Command::Open { duration_ms: Some(3000), paused: false, .. })));
    // 원본 보호: 내용·수정 시각 불변, 폴더에 부산물 없음
    assert_eq!(fingerprint(&original), before);
    assert_eq!(std::fs::read_dir(&fx.media).unwrap().count(), 1);
}

#[test]
fn t_player_open_04_service_opens_paused_when_autoplay_is_false() {
    let fx = fixture();
    let e = engine(&fx);
    let reply = e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    assert_eq!((reply.state, reply.time_ms), (ReplyState::Paused, 0));
    std::thread::sleep(Duration::from_millis(300));
    let ev: Vec<_> = e.events.lock().unwrap().iter().map(|e| (e.state, e.time_ms)).collect();
    assert!(ev.iter().all(|(s, _)| !matches!(s, EventState::Playing | EventState::Ended)), "{ev:?}");
    e.svc.control(ControlAction::Play).unwrap();
    assert!(wait_for(&e.events, 2000, |e| e.state == EventState::Playing).is_some());
}

#[test]
fn t_ipc_player_ranges_03_open_validates_ranges_before_asset_and_engine() {
    // Arrange: 엔진 없음 + 없는 자산 - 검증이 먼저면 invalid_input, 아니면 asset_offline/player_unavailable 이 나온다
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    for autoplay in [true, false] {
        for bad in [vec![range(5, 5)], vec![range(-1, 10)], vec![range(10, 5)], vec![range(0, 10), range(20, 20)]] {
            assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, canvas(), autoplay, Some(bad), None, None).unwrap_err()), "invalid_input", "{autoplay}");
        }
    }
    let many = |n: i64| (0..n).map(|i| range(i * 10, i * 10 + 5)).collect::<Vec<_>>();
    assert_eq!(code_of(&svc.open(&fx.reuse, fx.wav_id, canvas(), true, Some(many(1001)), None, None).unwrap_err()), "invalid_input");
    assert_eq!(code_of(&svc.open(&fx.reuse, fx.wav_id, canvas(), true, Some(many(1000)), None, None).unwrap_err()), "player_unavailable");
    // 유효한 구간 / null / 빈 목록은 검증을 통과해 다음 단계 오류를 낸다 (게이트 반대편)
    for ok in [None, Some(vec![]), Some(vec![range(0, 100)])] {
        assert_eq!(code_of(&svc.open(&fx.reuse, fx.wav_id, canvas(), true, ok.clone(), None, None).unwrap_err()), "player_unavailable", "{ok:?}");
        assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, canvas(), false, ok, None, None).unwrap_err()), "asset_offline");
    }
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn t_player_ranges_05_set_ranges_validates_arguments_before_the_engine() {
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    for bad in [vec![range(5, 5)], vec![range(-1, 10)], vec![range(10, 5)], vec![range(0, 10), range(20, 20)]] {
        assert_eq!(code_of(&svc.set_ranges(1, Some(bad)).unwrap_err()), "invalid_input");
    }
    let many = |n: i64| (0..n).map(|i| range(i * 10, i * 10 + 5)).collect::<Vec<_>>();
    svc.set_ranges(1, Some(many(1000))).unwrap();
    assert_eq!(code_of(&svc.set_ranges(1, Some(many(1001))).unwrap_err()), "invalid_input");
    svc.set_ranges(1, None).unwrap();
    svc.set_ranges(1, Some(vec![])).unwrap();
    svc.set_ranges(1, Some(vec![range(0, 100)])).unwrap();
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn t_player_open_14_service_sends_normalized_ranges_with_the_open_and_updates_them() {
    let fx = fixture();
    let e = engine(&fx);
    // 겹치는 구간은 병합·정렬되어 워커로 간다 (워커는 EDL 로 만든다)
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, Some(vec![range(5000, 6000), range(1000, 2000), range(1500, 2500)]), None, None).expect("open");
    assert!(e.script.commands().iter().any(|c| matches!(c, Command::Open { ranges: Some(r), .. } if r == &vec![(1000, 2500), (5000, 6000)])), "{:?}", e.script.commands());
    // 구간 해제 → 워커에 None
    e.svc.set_ranges(fx.wav_id, Some(vec![])).unwrap();
    assert!(e.script.commands().iter().any(|c| matches!(c, Command::SetRanges { ranges: None, .. })));
    // 다른 자산의 요청은 워커에 가지 않는다
    let n = e.script.commands().len();
    e.svc.set_ranges(fx.wav_id + 1, Some(vec![range(0, 10)])).unwrap();
    assert_eq!(e.script.commands().len(), n);
}

// 분류: Validation (command 인자 이름 계약 - JS 인자 이름은 Rust 인자 이름의 camelCase 이므로 시그니처가 바뀌면 프론트 invoke 가 조용히 깨진다.
// tauri 의 `test` feature 는 이 크레이트의 시험 실행 파일을 띄우지 못하게 해(STATUS_ENTRYPOINT_NOT_FOUND) mock IPC 대신 소스 시그니처를 고정한다)
#[test]
fn t_ipc_player_mpv_07_command_signatures_match_the_contract() {
    // Arrange: 도구가 만든 파일은 커밋 전까지 CRLF 일 수 있다 - 즉시 정규화
    let src = include_str!("player_commands.rs").replace("\r\n", "\n");
    let lib = include_str!("lib.rs").replace("\r\n", "\n");
    // Act / Assert: player_open — assetId, autoplay(필수), canvas, ranges(선택), startMs(선택, 이어보기)
    let open = src.split("pub async fn player_open(").nth(1).and_then(|s| s.split(") -> Result<OpenReply").next()).expect("player_open");
    for arg in ["asset_id: i64", "autoplay: bool", "canvas: CanvasSize", "ranges: Option<Vec<PlayRange>>", "start_ms: Option<u64>"] {
        assert!(open.contains(arg), "player_open must take `{arg}`: {open}");
    }
    assert!(!open.contains("rect") && !open.contains("Option<bool>"), "구 계약의 rect 가 남아 있다");
    for sig in [
        "pub async fn player_set_ranges(svc: Svc<'_>, asset_id: i64, ranges: Option<Vec<PlayRange>>) -> Result<(), CommandError>",
        "pub async fn player_control(svc: Svc<'_>, action: ControlAction) -> Result<(), CommandError>",
        "pub async fn player_seek(svc: Svc<'_>, ms: f64) -> Result<(), CommandError>",
        "pub async fn player_set_rate(svc: Svc<'_>, rate: f64) -> Result<(), CommandError>",
        "pub async fn player_set_volume(svc: Svc<'_>, volume: f64, muted: bool) -> Result<(), CommandError>",
        "pub async fn player_set_render_size(svc: Svc<'_>, width: u32, height: u32) -> Result<(), CommandError>",
        "pub async fn player_set_render_active(svc: Svc<'_>, active: bool) -> Result<(), CommandError>",
        "pub async fn player_release_slots(svc: Svc<'_>, release: serde_json::Value) -> Result<(), CommandError>",
        "pub async fn player_restart(svc: Svc<'_>) -> Result<OpenReply, CommandError>",
        "pub async fn player_close(svc: Svc<'_>) -> Result<(), CommandError>",
        "pub async fn player_available(svc: Svc<'_>) -> Result<bool, CommandError>",
    ] {
        assert!(src.contains(sig), "signature changed: {sig}");
    }
    // 제거된 command·이벤트가 다시 생기지 않았다 / 등록이 계약과 같다
    for gone in ["player_set_rect", "player_set_visible", "player_snapshot", "player://input"] {
        assert!(!src.contains(gone) && !lib.contains(&format!("player_commands::{gone}")), "removed contract item is back: {gone}");
    }
    for reg in ["player_available", "player_open", "player_control", "player_seek", "player_set_rate", "player_set_volume", "player_set_render_size", "player_set_render_active", "player_release_slots", "player_restart", "player_close", "player_set_ranges"] {
        assert!(lib.contains(&format!("player_commands::{reg},")), "not registered: {reg}");
    }
    assert!(lib.contains("player_commands::on_page_load_started(webview.app_handle())"));
    assert!(!lib.contains("player_input"), "player_input 은 삭제됐다");
    assert!(!std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/player_input.rs").exists());
}

#[test]
fn t_ipc_player_mpv_08_range_payload_shapes() {
    // 구간: camelCase 정수. 소수·누락·문자열은 command 진입 전에 역직렬화 실패 (프론트가 Math.round 해야 하는 이유)
    let r: PlayRange = serde_json::from_value(serde_json::json!({"startMs": 1000, "endMs": 2500})).unwrap();
    assert_eq!((r.start_ms, r.end_ms), (1000, 2500));
    for bad in [
        serde_json::json!({"startMs": 1000.5, "endMs": 2000}),
        serde_json::json!({"startMs": 1000}),
        serde_json::json!({"start_ms": 1000, "end_ms": 2000}),
        serde_json::json!({"startMs": "1000", "endMs": 2000}),
    ] {
        assert!(serde_json::from_value::<PlayRange>(bad.clone()).is_err(), "{bad}");
    }
}

// 분류: Business rule / Gate ON (렌더 켜짐 = 프론트 보임 AND 창 보임 — 꺼짐 게이트가 켜진 상태로 워커까지 간다)
#[test]
fn t_ipc_player_mpv_09_render_active_combines_the_page_and_the_window_and_resends_true() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    let sent = |v: bool| e.script.commands().iter().filter(|c| matches!(c, Command::SetRenderActive { active } if *active == v)).count();
    let settle = |n: usize, v: bool| {
        let end = Instant::now() + Duration::from_secs(2);
        while sent(v) < n && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        sent(v)
    };
    // 창 최소화 → 꺼짐
    e.svc.set_window_active(false);
    assert_eq!(settle(1, false), 1);
    // 프론트도 숨김 → 이미 꺼진 상태라 다시 보내지 않는다
    e.svc.set_render_active(false);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(sent(false), 1);
    // 창만 복원 → 프론트가 아직 숨김이라 켜지 않는다
    e.svc.set_window_active(true);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(sent(true), 0, "프론트가 숨겨져 있으면 켜지 않는다");
    // 프론트 복귀 → 켜진다. 이미 켜진 상태에서 다시 호출해도(WebGL 컨텍스트 복구) 한 번 더 보낸다 (현재 프레임 재게시)
    e.svc.set_render_active(true);
    assert_eq!(settle(1, true), 1);
    e.svc.set_render_active(true);
    assert_eq!(settle(2, true), 2);
}

// 분류: External / Business rule (워커가 연속으로 죽으면 player_restart_failed, [다시 시도] = player_restart)
#[test]
fn t_ipc_player_mpv_10_restart_failed_and_manual_restart_through_the_service() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).expect("open");
    e.script.fail_spawns.store(100, Ordering::SeqCst);
    e.svc.host.kill_worker_for_test();
    let crashed = wait_for(&e.events, 4000, |ev| ev.state == EventState::RestartFailed).expect("restartFailed");
    assert_eq!(serde_json::to_value(&crashed).unwrap()["errorCode"], "player_worker_crashed");
    assert_eq!(code_of(&e.svc.seek(10.0).unwrap_err()), "player_restart_failed");
    assert_eq!(code_of(&e.svc.control(ControlAction::StepForward).unwrap_err()), "player_restart_failed");
    // 다시 시도
    e.script.fail_spawns.store(0, Ordering::SeqCst);
    let r = e.svc.restart().expect("restart");
    assert_eq!(r.session_id, 2);
    e.svc.seek(10.0).unwrap();
    // 열린 세션이 없는데 restart → open_failed
    e.svc.close().unwrap();
    assert_eq!(code_of(&e.svc.restart().unwrap_err()), "open_failed");
}

// 분류: Boundary / External (숨기기 단축키 - 플레이어가 없거나 열린 미디어가 없어도 성공하고 아무 이벤트도 내지 않는다)
#[test]
fn t_hidekey_win_02_pause_for_hide_without_engine_or_media_is_a_no_op() {
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    svc.pause_for_hide();
    svc.pause_for_hide();
    assert!(events.lock().unwrap().is_empty());
    assert!(!svc.available());
    let e = engine(&fx);
    e.svc.pause_for_hide(); // 엔진은 있지만 열린 미디어 없음
    assert_eq!(e.script.spawns.load(Ordering::SeqCst), 0);
}

// 분류: External (페이지 재로드 - 엔진이 없어도 오류·대기 없이 반환) / Regression
#[test]
fn t_player_reload_01_without_engine_is_a_non_blocking_no_op() {
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    let started = Instant::now();
    svc.on_page_reload();
    svc.on_page_reload();
    assert!(started.elapsed() < Duration::from_millis(500), "must not wait for the player");
    assert!(events.lock().unwrap().is_empty());
}

// 분류: Regression (페이지가 다시 로드되면 열린 재생이 닫힌다 — React 상태는 사라져 player_close 가 오지 않는다)
#[test]
fn t_player_reload_02_reload_closes_the_player() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).expect("open");
    let started = Instant::now();
    e.svc.on_page_reload();
    assert!(started.elapsed() < Duration::from_millis(200), "비차단");
    // 같은 큐 순서의 동기 호출로 닫기가 처리됐음을 확인한다
    e.svc.seek(0.0).unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while !e.script.commands().contains(&Command::Close) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(e.script.commands().contains(&Command::Close));
    assert!(!e.svc.is_other_asset(1), "열린 자산을 잊었다");
}

// 분류: Regression / Concurrency (재로드 직후 새 페이지의 player_open 이 늦게 처리된 닫기에 닫히지 않는다 - 큐 순서)
#[test]
fn t_player_reload_03_open_after_reload_is_not_closed_by_the_pending_close() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).expect("open");
    e.svc.on_page_reload();
    let r = e.svc.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).expect("reopen");
    assert_eq!(r.session_id, 2);
    e.svc.seek(500.0).unwrap();
    assert!(wait_for(&e.events, 2000, |ev| ev.session_id == 2 && ev.time_ms == 500).is_some(), "새로 연 재생이 유지된다");
}

#[test]
fn t_ipc_eject_04_prepare_closes_the_player_only_when_the_open_asset_is_on_that_volume() {
    // Arrange: 열린 자산이 그 디스크에 있다 / 다른 디스크(모르는 자산)다
    let fx = fixture();
    let svc = Arc::new(no_engine(&fx).0);
    let hook_svc = Arc::clone(&svc);
    // 꺼낼 수 있는 디스크가 필요하다 - 실제 임시 폴더는 시스템 볼륨이라 `media` 를 USB 볼륨으로 보는 가짜 제공자를 쓴다
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&fx.pool),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(crate::volume_commands::eject_tests::MediaVolume(fx.media.clone())),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig { auto_excludes: vec![fx.data.clone()], ..IngestConfig::default() },
    );
    fx.pool
        .write(Priority::High, |c| {
            c.execute("UPDATE volumes SET volume_key = 'usb-test', kind = 'removable', last_mount_point = (SELECT display_path FROM managed_folders LIMIT 1)", [])?;
            Ok(())
        })
        .unwrap();
    ingest.set_eject_hook(Arc::new(move |scope| hook_svc.close_if_on_volume(scope)));
    // Act 1: 다른 자산이 열려 있다
    svc.set_current_asset_for_test(Some(9_999_999));
    ingest.prepare_volume_eject(1).unwrap();
    // Assert 1: 건드리지 않는다
    assert!(svc.is_other_asset(fx.wav_id), "an asset on another volume stays open");
    // Act 2: 이 디스크의 자산이 열려 있다
    ingest.resume_volume_watch(1).unwrap();
    svc.set_current_asset_for_test(Some(fx.wav_id));
    assert!(svc.is_other_asset(1_234));
    ingest.prepare_volume_eject(1).unwrap();
    // Assert 2: 닫혔다 (열린 자산 없음)
    assert!(!svc.is_other_asset(1_234), "the player was closed");
}

// 분류: Happy / Business rule (슬롯 반환 command — golden 메시지가 검증을 지나 현재 epoch 의 워커까지 간다)
#[test]
fn t_ipc_player_release_01_golden_release_message_is_forwarded_to_the_worker() {
    // Arrange: 열린 재생의 현재 epoch 로 golden 의 epoch 만 바꾼다
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    let epoch = e.svc.host.current_epoch();
    assert!(epoch > 0);
    let mut msg = fixture_json()["releaseMsg"].clone();
    msg["epoch"] = serde_json::json!(epoch);
    // Act
    e.svc.release_slots(msg).expect("valid release");
    // Assert: 워커가 같은 epoch·슬롯 목록으로 Release 를 받는다
    let got = || e.script.commands().into_iter().find_map(|c| if let Command::Release { epoch, slots } = c { Some((epoch, slots)) } else { None });
    let end = Instant::now() + Duration::from_secs(2);
    while got().is_none() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    let (got_epoch, slots) = got().expect("worker received Release");
    assert_eq!(got_epoch, epoch);
    assert_eq!(slots.len(), 2);
}

// 분류: Validation (fixture 의 거부 목록 + 경계 — command 는 invalid_input 으로 거부하고 워커에 아무것도 보내지 않는다)
#[test]
fn t_ipc_player_release_02_invalid_release_messages_are_rejected_without_reaching_the_worker() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    let epoch = e.svc.host.current_epoch();
    let fixt = fixture_json();
    let list = fixt["invalidReleaseMessages"].as_array().expect("list");
    assert!(!list.is_empty());
    // fixture 의 JSON 문자열 항목(= 문자열 메시지)도 값으로는 문자열이라 거부된다
    for v in list {
        assert_eq!(code_of(&e.svc.release_slots(v.clone()).unwrap_err()), "invalid_input", "{v}");
    }
    // 현재 epoch 로 보내도 형식 오류는 거부 — 알 수 없는 필드, 타입 오류, 음수·소수 epoch, 누락, 빈 값
    let ok = |epoch: serde_json::Value| serde_json::json!({"mam": "player.v1", "k": "release", "epoch": epoch, "slots": [{"slot": 0, "seq": 1}]});
    let mut extra = ok(serde_json::json!(epoch));
    extra["extra"] = serde_json::json!(1);
    let mut slot_extra = ok(serde_json::json!(epoch));
    slot_extra["slots"][0]["x"] = serde_json::json!(2);
    for bad in [extra, slot_extra, ok(serde_json::json!(-1)), ok(serde_json::json!(1.5)), ok(serde_json::json!("1")), serde_json::json!({}), serde_json::Value::Null, serde_json::json!([]), serde_json::json!(7)] {
        assert_eq!(code_of(&e.svc.release_slots(bad.clone()).unwrap_err()), "invalid_input", "{bad}");
    }
    // Assert: 거부된 메시지는 에러 본문에 값을 싣지 않고, 워커에 Release 가 가지 않는다
    let err = e.svc.release_slots(serde_json::json!({"secret": "Users-someone-secret"})).unwrap_err();
    assert!(!serde_json::to_string(&err).unwrap().contains("someone"));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.script.count(|c| matches!(c, Command::Release { .. })), 0);
}

// 분류: Boundary (epoch 0 = "없음" 은 거부, 슬롯 1·8개는 통과, 9개·slot 3 은 거부)
#[test]
fn t_ipc_player_release_03_epoch_zero_and_slot_count_boundaries() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    let epoch = e.svc.host.current_epoch();
    let msg = |epoch: u64, n: u64, slot: u32| serde_json::json!({"mam": "player.v1", "k": "release", "epoch": epoch, "slots": (0..n).map(|i| serde_json::json!({"slot": slot, "seq": i + 1})).collect::<Vec<_>>()});
    assert_eq!(code_of(&e.svc.release_slots(msg(0, 1, 0)).unwrap_err()), "invalid_input", "epoch 0");
    assert_eq!(code_of(&e.svc.release_slots(msg(epoch, 0, 0)).unwrap_err()), "invalid_input", "빈 슬롯");
    assert_eq!(code_of(&e.svc.release_slots(msg(epoch, 9, 0)).unwrap_err()), "invalid_input", "9 슬롯");
    assert_eq!(code_of(&e.svc.release_slots(msg(epoch, 1, 3)).unwrap_err()), "invalid_input", "slot 3");
    e.svc.release_slots(msg(epoch, 1, 2)).expect("1 슬롯");
    e.svc.release_slots(msg(epoch, 8, 0)).expect("8 슬롯");
}

// 분류: Concurrency / Business rule (지난 epoch 의 반환은 오류 없이 호스트가 버린다 — 재열기 직후 늦게 온 반환)
#[test]
fn t_ipc_player_release_04_stale_epoch_is_accepted_but_dropped_by_the_host() {
    let fx = fixture();
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    let epoch = e.svc.host.current_epoch();
    let stale = fixture_json()["releaseMsg"].clone(); // golden 의 epoch 3 — 현재 epoch 와 다르다
    assert_ne!(stale["epoch"].as_u64(), Some(epoch));
    e.svc.release_slots(stale).expect("a stale epoch is not an error");
    e.svc.release_slots(serde_json::json!({"mam": "player.v1", "k": "release", "epoch": epoch + 1, "slots": [{"slot": 0, "seq": 1}]})).expect("future epoch is not an error");
    // 큐 순서 보장용 동기 호출 뒤에도 Release 가 없다
    e.svc.seek(0.0).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(e.script.count(|c| matches!(c, Command::Release { .. })), 0, "{:?}", e.script.commands());
}

// 분류: External (엔진 없음/열린 재생 없음 — 막히거나 오류 없이 반환)
#[test]
fn t_ipc_player_release_05_without_engine_or_session_is_a_non_blocking_no_op() {
    let fx = fixture();
    let (svc, events) = no_engine(&fx);
    let started = Instant::now();
    svc.release_slots(fixture_json()["releaseMsg"].clone()).expect("no engine is not an error");
    assert!(started.elapsed() < Duration::from_millis(500));
    let e = engine(&fx);
    e.svc.release_slots(fixture_json()["releaseMsg"].clone()).expect("no session is not an error");
    assert_eq!(e.script.spawns.load(Ordering::SeqCst), 0, "반환이 워커를 띄우지 않는다");
    assert!(events.lock().unwrap().is_empty());
}

// 분류: Happy (startMs 가 워커 Open 으로 간다) / Boundary (생략·0 은 그대로) / Validation (i64 범위 밖 거부 - 엔진·자산 접근보다 먼저)
#[test]
fn t_resume_player_01_open_sends_start_ms_to_the_worker_and_validates_it_first() {
    let fx = fixture();
    // Arrange / Act: 시작 위치 있음 / 없음
    let e = engine(&fx);
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, Some(12_345)).expect("open");
    e.svc.open(&fx.reuse, fx.wav_id, canvas(), false, None, None, None).expect("open");
    // Assert
    let opens: Vec<_> = e.script.commands().into_iter().filter_map(|c| match c { Command::Open { start_ms, .. } => Some(start_ms), _ => None }).collect();
    assert_eq!(opens, [Some(12_345), None]);
    // Validation: 범위 밖 값은 없는 자산 오류보다 먼저 invalid_input
    let (svc, _events) = no_engine(&fx);
    assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, canvas(), true, None, None, Some(u64::MAX)).unwrap_err()), "invalid_input");
    assert_eq!(code_of(&svc.open(&fx.reuse, 9_999_999, canvas(), true, None, None, Some(1000)).unwrap_err()), "asset_offline");
}
