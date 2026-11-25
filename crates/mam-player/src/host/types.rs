//! 호스트 공개 타입 — 설정·타임아웃·열기 요청/결과.

use std::path::PathBuf;
use std::time::Duration;

use crate::protocol::RangeMs;

/// 계약 §7 의 타임아웃 표 (시험은 짧게 줄여 쓴다)
#[derive(Debug, Clone)]
pub struct Timeouts {
    /// `open`/`restart` 응답 (재시도 없음 — 넘으면 워커를 죽이고 실패로 센다)
    pub open: Duration,
    /// 멱등 명령(seek·play·pause·rate·volume·ranges·subtitle)·`Attach`·`Close` 응답. 멱등 명령은 한 번 더 시도한다
    pub command: Duration,
    /// 세션이 열린 동안 워커가 아무 메시지(하트비트 포함)도 보내지 않은 최대 시간 → 핸들로 종료 후 재시작
    pub silence: Duration,
    /// 워커 시작(파이프 연결 + `Ready`)
    pub spawn_ready: Duration,
    /// 재시작 백오프 (1·2·3번째 시도 전)
    pub backoff: [Duration; 3],
    /// 종료 시 `Quit` 후 기다리는 시간
    pub quit: Duration,
    /// `PlaybackRestart` 뒤 첫 프레임을 더 기다리는 유예 (그 뒤에는 프레임 없이 열기를 마친다)
    pub frame_grace: Duration,
    /// 복원 전체 제한
    pub restore: Duration,
    /// 렌더 크기 변경(epoch 교체) 최소 간격
    pub resize_interval: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            open: Duration::from_secs(20),
            command: Duration::from_secs(3),
            silence: Duration::from_secs(5),
            spawn_ready: Duration::from_secs(10),
            backoff: [Duration::ZERO, Duration::from_millis(250), Duration::from_secs(1)],
            quit: Duration::from_millis(800),
            frame_grace: Duration::from_millis(500),
            restore: Duration::from_secs(20),
            resize_interval: Duration::from_millis(250),
        }
    }
}

/// 자동 재시작 최대 시도 (연속 실패 3회 → `restartFailed`)
pub const MAX_RESTARTS: u8 = 3;

#[derive(Debug, Clone, Default)]
pub struct HostConfig {
    /// 워커 실행 파일 / libmpv 폴더 — 둘 중 하나라도 없으면 `is_available() == false`
    pub worker_exe: Option<PathBuf>,
    pub libmpv_dir: Option<PathBuf>,
    /// 워커 stderr 를 호스트 stderr 로 내보낸다 (진단용)
    pub inherit_stderr: bool,
    /// 워커 환경변수 추가분 (시험용)
    pub worker_env: Vec<(String, String)>,
    pub timeouts: Timeouts,
}

/// 호스트가 이미 검증·해석한 열기 요청 (프론트는 경로를 보내지 않는다)
#[derive(Debug, Clone)]
pub struct OpenRequest {
    pub asset_id: i64,
    pub path: PathBuf,
    pub autoplay: bool,
    /// 캔버스 장치 픽셀 크기 — 호스트가 `fit_render_size` 로 렌더 크기를 정한다
    pub canvas: (u32, u32),
    /// 정렬·병합된 구간 (없으면 None)
    pub ranges: Option<Vec<RangeMs>>,
    /// 자산 행의 소스 길이(ms). 구간을 쓰는데 없으면 워커가 먼저 열어 길이를 얻는다
    pub duration_hint_ms: Option<i64>,
    /// 열기 직후 시작할 소스 시각(ms, 첫 프레임 = 0). 이어보기 - 첫 프레임을 보이기 전에 워커가 이동한다. 없거나 0 이면 처음부터
    pub start_ms: Option<i64>,
}

/// `player_open`/`player_restart` 응답 — 시각은 소스 타임라인(0 = 파일 첫 프레임)
#[derive(Debug, Clone, PartialEq)]
pub struct OpenInfo {
    pub session_id: u64,
    pub length_ms: Option<i64>,
    pub fps: Option<f64>,
    pub paused: bool,
    pub time_ms: i64,
}
