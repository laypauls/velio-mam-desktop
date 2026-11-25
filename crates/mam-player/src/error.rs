//! 플레이어 오류 — 엔진(워커/libmpv) 부재·실행 실패·열기 실패·시간 초과·재시작 상태를 구분한다. 패닉하지 않는다.
//! 메시지에 사용자 경로를 싣지 않는다 (command 경계가 사용자 메시지 키로 바꾼다).

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    /// 워커 실행 파일 또는 libmpv 를 찾지 못했다 (동봉 누락·잘못된 환경변수)
    #[error("player worker or libmpv not found: {0}")]
    WorkerNotFound(&'static str),
    /// 워커를 띄우거나 파이프에 연결하지 못했다
    #[error("cannot start the player worker: {0}")]
    SpawnFailed(String),
    /// 워커와 호스트의 프로토콜 버전이 다르다
    #[error("player worker protocol mismatch (worker {worker}, host {host})")]
    ProtocolMismatch { worker: u32, host: u32 },
    /// WebView2 가 SharedBuffer 를 지원하지 않는다 / 버퍼 생성 실패
    #[error("frame surface is unavailable: {0}")]
    SurfaceUnavailable(String),
    /// 워커가 파일을 열지 못했다 (손상·미지원 형식·읽기 오류)
    #[error("cannot open the media: {0}")]
    OpenFailed(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// 워커가 명령을 거부했다
    #[error("player command failed: {0}")]
    CommandFailed(String),
    /// 열린 미디어가 없다
    #[error("no media is open")]
    NoMedia,
    /// 워커가 제한 시간(재시도 포함) 안에 응답하지 않았다 — 워커는 종료되고 재시작된다
    #[error("player worker did not reply in time")]
    Timeout,
    /// 복원 중이라 지금은 할 수 없는 명령 (프레임 이동)
    #[error("player is restarting")]
    Restarting,
    /// 워커가 연속으로 죽어 재시작을 포기했다 — 열기/재시작/닫기만 받는다
    #[error("player restart failed repeatedly")]
    RestartFailed,
    /// 호스트 스레드가 종료됐다 (앱 종료 중)
    #[error("player host is gone")]
    HostGone,
    /// 사용자가 닫기·페이지 재로드로 대기 중인 열기를 취소했다
    #[error("cancelled")]
    Cancelled,
}

impl PlayerError {
    /// 재생 엔진 자체를 쓸 수 없다 — 호출자는 "내장 플레이어 없음" 으로 보고 외부 앱 열기로 폴백한다
    pub fn is_engine_unavailable(&self) -> bool {
        matches!(self, Self::WorkerNotFound(_) | Self::SpawnFailed(_) | Self::ProtocolMismatch { .. } | Self::SurfaceUnavailable(_))
    }
}
