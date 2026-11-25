//! 콘텐츠 보기 플레이어 command (libmpv 워커 프로세스 + WebView2 SharedBuffer).
//! 재생 엔진은 `mam_player::host::PlayerHost`(감독자 스레드 + 워커 프로세스)에 있고, 여기서는 자산 해석·인자 검증·오류 코드 변환·이벤트 전달만 한다.
//! 계약 정본: `docs/requests/2026-11/20261119-192800-architect-libmpv-ipc-contract.md` §1~§3, TS 쪽 `src/lib/ipc-contract-player.ts`.
//!
//! - 원본은 워커가 읽기 전용으로만 연다. 프론트는 경로·핸들을 보내지 않는다 (자산 id 만)
//! - 오류 코드: `player_unavailable`(엔진 없음) / `asset_offline` / `open_failed` / `player_timeout` / `player_restarting` / `player_restart_failed`
//! - `player_release_slots` 를 뺀 모든 command 는 `spawn_blocking` — 감독자 호출이 응답을 기다리므로 async 작업자를 막지 않는다.
//!   슬롯 반환은 프레임마다 오고 호스트 호출이 비차단이라 직접 부른다 (WebView `postMessage` 는 wry 핸들러가 가로채 쓸 수 없다)

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mam_core::reuse::Reuse;
use mam_core::scan::IngestError;
use mam_player::host::{EventSink, HostConfig, OpenInfo, OpenRequest, PlayerEvent, PlayerHost};
use mam_player::protocol::{ControlAction, validate_release_value};
use mam_player::{PlayRange, PlayerError, normalize_ranges, valid_ranges};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::error::{CommandError, ErrorCode};
use crate::player_subtitle::SubtitleFiles;
use crate::storage_commands::Storage;

/// 계약 `PLAYER_STATE_EVENT`
pub const PLAYER_STATE_EVENT: &str = "player://state";
const SHUTDOWN_WAIT: Duration = Duration::from_millis(800);
/// 캔버스·렌더 크기 한 변의 허용 범위 (장치 픽셀)
const MAX_SIDE: u32 = 16384;

/// 계약 `CanvasSize` — 캔버스 장치 픽셀 (사각형이 아니라 크기만 — 위치·DPI 동기화가 필요 없다)
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanvasSize {
    pub width: u32,
    pub height: u32,
}

/// 계약 `PlayerOpenResult` 의 상태
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ReplyState {
    Playing,
    Paused,
}

/// `player_open`/`player_restart` 반환 — 시각은 소스 타임라인(0 = 파일 첫 프레임). 프론트가 첫 상태를 여기서 얻는다 (첫 프레임 뒤 이벤트 없음)
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenReply {
    pub session_id: u64,
    pub length_ms: Option<i64>,
    pub fps: Option<f64>,
    pub state: ReplyState,
    pub time_ms: i64,
}

impl From<OpenInfo> for OpenReply {
    fn from(i: OpenInfo) -> Self {
        Self { session_id: i.session_id, length_ms: i.length_ms, fps: i.fps, state: if i.paused { ReplyState::Paused } else { ReplyState::Playing }, time_ms: i.time_ms }
    }
}

pub struct PlayerService {
    host: Arc<PlayerHost>,
    /// 외부 자막용 임시 SRT (`cache/player-subtitles`) - 열기·닫기가 끝난 뒤 지운다
    subs: SubtitleFiles,
    /// 열려 있는 자산 (열기 성공 ~ 닫기). 모르면 `None` - 다른 자산의 자막·구간 요청을 걸러내는 데만 쓴다
    current_asset: Mutex<Option<i64>>,
    /// 렌더 켜짐 = 프론트가 보이는 중 AND 창이 최소화·숨김이 아님 (둘 중 하나라도 꺼지면 워커 렌더를 멈춘다)
    js_active: AtomicBool,
    window_active: AtomicBool,
    last_active: AtomicBool,
}

/// 오류 변환 - 사용자 경로·내부 문구는 싣지 않는다. 원인은 로그에만 남긴다
pub(crate) fn open_error(e: PlayerError) -> CommandError {
    if e.is_engine_unavailable() {
        return CommandError::logged(ErrorCode::PlayerUnavailable, "player engine is not available", &e);
    }
    match e {
        PlayerError::Timeout => CommandError::logged(ErrorCode::PlayerTimeout, "the player did not respond in time", &e),
        PlayerError::RestartFailed => CommandError::logged(ErrorCode::PlayerRestartFailed, "the player stopped repeatedly", &e),
        e => CommandError::logged(ErrorCode::OpenFailed, "cannot open the media for playback", &e),
    }
}

pub(crate) fn command_error(e: PlayerError) -> CommandError {
    match e {
        PlayerError::NoMedia => CommandError::new(ErrorCode::NotFound, "no media is open"),
        PlayerError::Timeout => CommandError::logged(ErrorCode::PlayerTimeout, "the player did not respond in time", &e),
        PlayerError::Restarting => CommandError::new(ErrorCode::PlayerRestarting, "the player is restarting"),
        PlayerError::RestartFailed => CommandError::logged(ErrorCode::PlayerRestartFailed, "the player stopped repeatedly", &e),
        PlayerError::InvalidArgument(_) => CommandError::logged(ErrorCode::InvalidInput, "invalid argument", &e),
        e if e.is_engine_unavailable() => CommandError::logged(ErrorCode::PlayerUnavailable, "player engine is not available", &e),
        e => CommandError::logged(ErrorCode::Internal, "player command failed", &e),
    }
}

fn invalid(what: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, format!("invalid {what}"))
}

/// 구간 인자 검증(개수 <= 1000, `start >= 0`, `end > start`) + 정렬·병합 - `player_open`·`player_set_ranges` 가 같이 쓴다. 빈 목록 = 구간 없음
fn to_pairs(ranges: Option<Vec<PlayRange>>) -> Result<Option<Vec<(i64, i64)>>, CommandError> {
    match ranges {
        None => Ok(None),
        Some(v) if !valid_ranges(&v) => Err(invalid("ranges")),
        Some(v) => Ok(Some(normalize_ranges(v.into_iter().map(|r| (r.start_ms, r.end_ms)).collect())).filter(|r| !r.is_empty())),
    }
}

fn check_side(v: u32) -> bool {
    (1..=MAX_SIDE).contains(&v)
}

/// 개발(디버그) 빌드 전용 - libmpv 를 찾을 저장소 폴더 (컴파일 시점 경로). 릴리스 빌드는 이 경로를 담지 않는다
#[cfg(debug_assertions)]
fn dev_dirs() -> Vec<PathBuf> {
    let m = env!("CARGO_MANIFEST_DIR");
    vec![PathBuf::from(m).join("binaries/libmpv"), PathBuf::from(m).join("../temp/poc-mpv/lib")]
}

#[cfg(not(debug_assertions))]
fn dev_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// 자산 행의 소스 길이(ms) — 구간 재생(EDL)의 길이 힌트. 없으면 워커가 먼저 열어 얻는다
fn duration_hint(storage: &Storage, asset_id: i64) -> Option<i64> {
    let info = storage.pool.read(|c| Ok(mam_core::processing::media_info(c, asset_id))).ok()?.ok()??;
    info.duration_ms.filter(|d| *d > 0)
}

impl PlayerService {
    /// 시험·조립용 - 호스트를 직접 받는다
    pub fn with_host(host: Arc<PlayerHost>, cache_dir: PathBuf) -> Self {
        Self {
            host,
            subs: SubtitleFiles::new(&cache_dir),
            current_asset: Default::default(),
            js_active: AtomicBool::new(true),
            window_active: AtomicBool::new(true),
            last_active: AtomicBool::new(true),
        }
    }

    /// 앱 조립: 워커·libmpv 위치 탐색(환경변수 -> exe 옆 -> 디버그 빌드만 저장소 폴더)과 WebView2 표면 연결. 워커는 첫 `player_open` 에서 띄운다
    pub fn new(app: &tauri::AppHandle, cache_dir: PathBuf) -> Self {
        let dev = dev_dirs();
        let worker = mam_player::host::locate::locate_worker_default(&[]);
        let libmpv = mam_player::host::locate::locate_libmpv_default(&dev);
        tracing::info!(worker = ?worker.as_deref(), libmpv = ?libmpv.as_deref(), "player engine located (None = missing; the built-in player is disabled)");
        let surface = Arc::new(crate::player_surface::TauriSurface::new(app.clone()));
        let emitter = app.clone();
        let sink: EventSink = Arc::new(move |e: PlayerEvent| {
            if let Err(err) = emitter.emit(PLAYER_STATE_EVENT, &e) {
                tracing::warn!(%err, "cannot emit player state");
            }
        });
        let cfg = HostConfig { worker_exe: worker, libmpv_dir: libmpv, inherit_stderr: std::env::var_os("MAM_PLAYER_LOG").is_some(), ..HostConfig::default() };
        let host = Arc::new(PlayerHost::new(cfg, surface, sink));
        Self::with_host(host, cache_dir)
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<i64>> {
        self.current_asset.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 다른 자산이 열려 있다고 확실히 아는가 - 그렇다면 엔진이 어차피 무시할 요청이다 (아니면 `false`: 모르면 걸러내지 않는다)
    pub fn is_other_asset(&self, asset_id: i64) -> bool {
        self.current().is_some_and(|cur| cur != asset_id)
    }

    #[cfg(test)]
    pub(crate) fn set_current_asset_for_test(&self, asset_id: Option<i64>) {
        *self.current() = asset_id;
    }

    pub fn available(&self) -> bool {
        self.host.is_available()
    }

    /// 자산을 해석(오프라인·파일 없음이면 `asset_offline`)한 뒤 읽기 전용으로 연다. `autoplay` 가 아니면 첫 프레임에서 정지한 채(`paused`) 둔다.
    /// `ranges`(선택)는 `set_ranges` 와 같은 검증을 거쳐 열기와 함께 적용된다. 인자 검증은 자산 조회·엔진 접근보다 먼저다
    pub fn open(&self, reuse: &Reuse, asset_id: i64, canvas: CanvasSize, autoplay: bool, ranges: Option<Vec<PlayRange>>, hint: Option<i64>, start_ms: Option<u64>) -> Result<OpenReply, CommandError> {
        let ranges = to_pairs(ranges)?;
        let start_ms = start_ms.map(|ms| i64::try_from(ms).map_err(|_| invalid("startMs"))).transpose()?;
        if !check_side(canvas.width) || !check_side(canvas.height) {
            return Err(invalid("canvas"));
        }
        let path = match reuse.original_path(asset_id) {
            Ok(p) => p,
            Err(IngestError::Offline | IngestError::NotFound) => {
                return Err(CommandError::new(ErrorCode::AssetOffline, "the original file is offline or missing"));
            }
            Err(e) => return Err(e.into()),
        };
        let opened = self.host.open(OpenRequest { asset_id, path, autoplay, canvas: (canvas.width, canvas.height), ranges, duration_hint_ms: hint, start_ms });
        self.subs.clear(); // 이전 플레이어가 자막 파일 핸들을 놓은 뒤 (성공·실패 모두)
        *self.current() = opened.is_ok().then_some(asset_id);
        opened.map(OpenReply::from).map_err(open_error)
    }

    pub fn control(&self, action: ControlAction) -> Result<(), CommandError> {
        self.host.control(action).map_err(command_error)
    }

    pub fn seek(&self, ms: f64) -> Result<(), CommandError> {
        if !ms.is_finite() {
            return Err(invalid("ms"));
        }
        self.host.seek(ms.max(0.0) as i64).map_err(command_error)
    }

    pub fn set_rate(&self, rate: f64) -> Result<(), CommandError> {
        if !rate.is_finite() || !(0.1..=16.0).contains(&rate) {
            return Err(invalid("rate"));
        }
        self.host.set_rate(rate as f32).map_err(command_error)
    }

    pub fn set_volume(&self, volume: f64, muted: bool) -> Result<(), CommandError> {
        if !volume.is_finite() {
            return Err(invalid("volume"));
        }
        self.host.set_volume(volume.round().clamp(0.0, 100.0) as i32, muted).map_err(command_error)
    }

    /// 캔버스 장치 픽셀 크기 (120ms 디바운스된 값) — 렌더 크기를 바꾼다. 열린 미디어가 없으면 기억만 한다
    pub fn set_render_size(&self, width: u32, height: u32) -> Result<(), CommandError> {
        if !check_side(width) || !check_side(height) {
            return Err(invalid("size"));
        }
        self.host.set_render_size(width, height).map_err(command_error)
    }

    /// 프론트의 보임 여부 (`document.hidden`·보기 숨김). 창 최소화와 합쳐 워커에 알린다. UI 스레드에서 불러도 막히지 않는다
    pub fn set_render_active(&self, active: bool) {
        self.js_active.store(active, Ordering::Release);
        self.sync_render_active();
    }

    /// JS 가 그린 프레임의 슬롯 반환 — 형식 검증(`validate_release_value`) 뒤 호스트로 넘긴다. 절대 막지 않는다.
    /// 지난 epoch 의 반환은 호스트가 조용히 버린다 (오류 아님). 본문은 로그에 남기지 않는다
    pub fn release_slots(&self, release: serde_json::Value) -> Result<(), CommandError> {
        let msg = validate_release_value(release).map_err(|_| invalid("release"))?;
        if msg.epoch == 0 {
            return Err(invalid("release"));
        }
        let slots: Vec<(u32, u64)> = msg.slots.iter().map(|s| (s.slot, s.seq)).collect();
        self.host.release_slots(msg.epoch, &slots);
        Ok(())
    }

    /// 창 최소화·트레이 숨김 (`WindowEvent`) — 프론트 JS 는 최소화로 멈추지 않는다 (PoC: rAF 143Hz 유지)
    pub fn set_window_active(&self, active: bool) {
        self.window_active.store(active, Ordering::Release);
        self.sync_render_active();
    }

    fn sync_render_active(&self) {
        let eff = self.js_active.load(Ordering::Acquire) && self.window_active.load(Ordering::Acquire);
        if self.last_active.swap(eff, Ordering::AcqRel) != eff || eff {
            // `true` 는 값이 같아도 다시 보낸다 — 워커가 현재 프레임을 다시 게시한다 (WebGL 컨텍스트 복구·접힌 스테이지 복귀)
            self.host.set_render_active(eff);
        }
    }

    /// 재생 구간 (`null`/빈 목록 = 구간 모드 끔). 열린 미디어가 없거나 `asset_id` 가 현재 자산과 다르면 무시한다
    pub fn set_ranges(&self, asset_id: i64, ranges: Option<Vec<PlayRange>>) -> Result<(), CommandError> {
        let pairs = to_pairs(ranges)?;
        self.host.set_ranges(asset_id, pairs).map_err(command_error)
    }

    /// 외부 자막 선택 (`None` = 끔). SRT 를 MAM 캐시에 쓴 뒤 워커에 넘긴다. 임시 파일은 워커가 놓았다는 응답 뒤에만 지운다
    pub fn set_subtitle(&self, asset_id: i64, sub: Option<(String, Vec<mam_core::subtitle::Cue>)>) -> Result<(), CommandError> {
        let path = match sub {
            Some((id, cues)) => Some(self.subs.prepare(asset_id, &id, &cues)?),
            None => None,
        };
        let clearing = path.is_none();
        let r = self.host.set_subtitle(asset_id, path).map_err(command_error);
        if clearing && r.is_ok() {
            self.subs.clear();
        }
        r
    }

    /// 워커를 다시 띄워 기억한 세션을 복원한다 (3회 연속 실패 뒤 사용자가 누르는 [다시 시도])
    pub fn restart(&self) -> Result<OpenReply, CommandError> {
        let r = self.host.restart().map_err(open_error)?;
        Ok(r.into())
    }

    pub fn close(&self) -> Result<(), CommandError> {
        let closed = self.host.close();
        *self.current() = None;
        self.subs.clear(); // 워커가 파일을 놓았다는 응답(또는 타임아웃) 뒤
        closed.map_err(command_error)
    }

    /// 디스크 꺼내기 준비: 열려 있는 자산이 그 디스크에 있으면 재생을 닫는다 (워커가 원본 파일 핸들을 놓을 때까지 기다린다).
    /// 열린 자산이 없거나 다른 디스크면 아무것도 하지 않는다. 닫기 실패는 로그만 (준비는 계속된다)
    pub fn close_if_on_volume(&self, scope: &mam_core::ingest::EjectScope) {
        let on_volume = self.current().is_some_and(|asset_id| scope.contains_asset(asset_id));
        if on_volume && let Err(e) = self.close() {
            tracing::warn!(code = ?e.code(), "eject: cannot close the player");
        }
    }

    /// 숨기기 단축키용 - 재생 중이면 **사용자 일시정지**로 멈춘다. 열린 미디어가 없거나 엔진이 없으면 아무 일도 하지 않으며,
    /// 실패해도 창 숨김을 막지 않도록 오류를 삼킨다(로그만)
    pub fn pause_for_hide(&self) {
        if let Err(e) = self.control(ControlAction::Pause) {
            tracing::warn!(code = ?e.code(), "hide: cannot pause the player");
        }
    }

    /// WebView 페이지 로드가 시작될 때(새로고침·개발 서버 재로드) - React 상태가 초기화되며 `player_close` 가 오지 않는다.
    /// 대기 중인 열기를 취소하고 닫기를 큐에 넣는다 (응답을 기다리지 않는다 - UI 스레드)
    pub fn on_page_reload(&self) {
        self.host.close_nowait();
        *self.current() = None;
    }

    /// 앱 종료(`RunEvent::Exit`): 워커에 종료를 알리고 정리한다
    pub fn shutdown(&self) {
        self.host.shutdown(SHUTDOWN_WAIT);
        self.subs.remove_all();
    }
}

type Svc<'a> = State<'a, Arc<PlayerService>>;

/// 감독자 호출은 응답을 기다리므로 `spawn_blocking` 에서 실행한다 (async 작업자를 막지 않는다)
async fn blocking<T, F>(svc: &Svc<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&PlayerService) -> Result<T, CommandError> + Send + 'static,
{
    let svc = Arc::clone(svc);
    tauri::async_runtime::spawn_blocking(move || f(&svc)).await.map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })?
}

#[tauri::command]
pub async fn player_available(svc: Svc<'_>) -> Result<bool, CommandError> {
    blocking(&svc, |s| Ok(s.available())).await
}

#[tauri::command]
pub async fn player_open(
    svc: Svc<'_>,
    reuse: State<'_, Arc<Reuse>>,
    storage: State<'_, Arc<Storage>>,
    asset_id: i64,
    autoplay: bool,
    canvas: CanvasSize,
    ranges: Option<Vec<PlayRange>>,
    start_ms: Option<u64>,
) -> Result<OpenReply, CommandError> {
    let (reuse, storage) = (Arc::clone(&reuse), Arc::clone(&storage));
    blocking(&svc, move |s| {
        let hint = duration_hint(&storage, asset_id);
        s.open(&reuse, asset_id, canvas, autoplay, ranges, hint, start_ms)
    })
    .await
}

#[tauri::command]
pub async fn player_set_ranges(svc: Svc<'_>, asset_id: i64, ranges: Option<Vec<PlayRange>>) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.set_ranges(asset_id, ranges)).await
}

#[tauri::command]
pub async fn player_control(svc: Svc<'_>, action: ControlAction) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.control(action)).await
}

#[tauri::command]
pub async fn player_seek(svc: Svc<'_>, ms: f64) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.seek(ms)).await
}

#[tauri::command]
pub async fn player_set_rate(svc: Svc<'_>, rate: f64) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.set_rate(rate)).await
}

#[tauri::command]
pub async fn player_set_volume(svc: Svc<'_>, volume: f64, muted: bool) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.set_volume(volume, muted)).await
}

#[tauri::command]
pub async fn player_set_render_size(svc: Svc<'_>, width: u32, height: u32) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.set_render_size(width, height)).await
}

#[tauri::command]
pub async fn player_set_render_active(svc: Svc<'_>, active: bool) -> Result<(), CommandError> {
    blocking(&svc, move |s| {
        s.set_render_active(active);
        Ok(())
    })
    .await
}

/// 슬롯 반환 — 프레임마다 불리는 경로라 `blocking` 을 거치지 않는다 (호스트 호출은 비차단: epoch 검사 + `try_send`)
#[tauri::command]
pub async fn player_release_slots(svc: Svc<'_>, release: serde_json::Value) -> Result<(), CommandError> {
    svc.release_slots(release)
}

#[tauri::command]
pub async fn player_restart(svc: Svc<'_>) -> Result<OpenReply, CommandError> {
    blocking(&svc, |s| s.restart()).await
}

#[tauri::command]
pub async fn player_close(svc: Svc<'_>) -> Result<(), CommandError> {
    blocking(&svc, |s| s.close()).await
}

/// `Builder::on_page_load` 의 `PageLoadEvent::Started`(메인 웹뷰)에서 부른다 - 대기 중인 열기 취소·닫기. 플레이어 서비스가 아직 없으면 무시
pub fn on_page_load_started(app: &tauri::AppHandle) {
    if let Some(svc) = app.try_state::<Arc<PlayerService>>() {
        svc.on_page_reload();
    }
}

/// 창 최소화·복원·트레이 숨김/표시 → 워커 렌더 켜짐/꺼짐 (`WindowEvent::Resized` + 숨김 경로가 부른다)
pub fn set_window_active(app: &tauri::AppHandle, active: bool) {
    if let Some(svc) = app.try_state::<Arc<PlayerService>>() {
        svc.set_window_active(active);
    }
}

/// `WindowEvent::Resized` 에서 최소화 여부를 보고 렌더를 켜고 끈다
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    if matches!(event, tauri::WindowEvent::Resized(_)) {
        let minimized = window.is_minimized().unwrap_or(false);
        set_window_active(window.app_handle(), !minimized);
    }
}

#[cfg(test)]
#[path = "player_commands_tests.rs"]
pub(crate) mod tests;

