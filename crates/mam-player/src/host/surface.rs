//! 프레임 표면 추상화 — WebView2 SharedBuffer 와 프레임 알림. `mam-player` 는 `webview2-com` 에 의존하지 않고,
//! 앱(`src-tauri/player_surface.rs`)이 이 trait 를 구현한다. 시험은 일반 파일 매핑으로 만든 가짜 표면을 쓴다 (`host::testing`).

use super::proc::RawProcessHandle;
use crate::protocol::FrameMsgOut;

/// 한 세대(epoch)의 버퍼 3개 — `handles` 는 **워커 프로세스 안에서 유효한** 파일 매핑 핸들 값
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankAttach {
    pub epoch: u64,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub size: u64,
    pub handles: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SurfaceError {
    /// WebView2 런타임이 SharedBuffer 를 지원하지 않는다 (`ICoreWebView2_17`/`Environment12` 캐스트 실패)
    #[error("shared buffers are not supported")]
    Unsupported,
    /// UI 스레드가 제한 시간(2초) 안에 응답하지 않았다
    #[error("ui thread did not respond in time")]
    Timeout,
    #[error("{0}")]
    Failed(String),
}

pub trait FrameSurface: Send + Sync {
    /// 버퍼 3개를 만들어 JS 에 게시하고 워커로 핸들을 복제한다. **감독자 스레드에서 호출**되며, 구현은 UI 스레드에 일을 넘기고
    /// 그 스레드를 막는다 (2초 제한). UI 스레드 자신에서 부르지 않는다
    fn create_bank(&self, epoch: u64, w: u32, h: u32, worker_process: RawProcessHandle) -> Result<BankAttach, SurfaceError>;
    /// 호스트가 쥔 버퍼 참조를 닫는다 (비차단). JS·워커가 쥔 뷰는 각자 해제될 때까지 유효하다
    fn retire_bank(&self, epoch: u64);
    /// 프레임 알림을 JS 로 보낸다 (비차단 — UI 스레드로 넘기고 즉시 반환)
    fn post_frame(&self, msg: &FrameMsgOut);
    /// SharedBuffer 를 쓸 수 있는가 (결과는 캐시해도 된다). 거짓이면 `player_available == false`
    fn capabilities_ok(&self) -> bool;
}
