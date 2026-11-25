//! 호출 오류. 메시지에는 플러그인 소스·비밀을 싣지 않는다. command 경계(P4~P6)가 `key()` 로 사용자 메시지 키를 만든다.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvokeError {
    #[error("plugin disabled after repeated worker faults")]
    Disabled,
    #[error("worker could not be started: {0}")]
    Start(String),
    #[error("plugin failed to initialize: {0}")]
    Init(String),
    #[error("handler not registered: {0}")]
    NotRegistered(String),
    #[error("call timed out")]
    Timeout,
    #[error("worker exited unexpectedly")]
    Crashed,
    #[error("worker protocol violation: {0}")]
    Protocol(String),
    #[error("worker output line exceeded the size limit")]
    Oversize,
    #[error("worker produced too many messages in one call")]
    Flood,
    #[error("resource limit exceeded: {0}")]
    Limit(String),
    #[error("plugin error: {0}")]
    Script(String),
    #[error("plugin error ({code}): {message}")]
    Plugin { code: String, message: String },
}

impl InvokeError {
    /// 워커를 버리고 차단기에 기록해야 하는 결함인가. 스크립트 예외·미등록 핸들러·시작 실패(실행 파일 문제)는 아니다
    pub fn is_fault(&self) -> bool {
        matches!(
            self,
            Self::Init(_)
                | Self::Timeout
                | Self::Crashed
                | Self::Protocol(_)
                | Self::Oversize
                | Self::Flood
                | Self::Limit(_)
        )
    }

    /// 사용자 메시지 키 (i18n)
    pub fn key(&self) -> &'static str {
        match self {
            Self::Disabled => "plugin.error.disabled",
            Self::Start(_) => "plugin.error.start",
            Self::Init(_) => "plugin.error.init",
            Self::NotRegistered(_) => "plugin.error.notRegistered",
            Self::Timeout => "plugin.error.timeout",
            Self::Crashed => "plugin.error.crashed",
            Self::Protocol(_) | Self::Oversize | Self::Flood => "plugin.error.protocol",
            Self::Limit(_) => "plugin.error.limit",
            Self::Script(_) | Self::Plugin { .. } => "plugin.error.script",
        }
    }
}
