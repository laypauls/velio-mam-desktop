//! P3 AI 앱 연결 — 설정·동의·키·로컬 모델·분석 작업·이벤트 (정책 PA1~PA9: `docs/requests/2026-10/20261029-005836-p3-policies-and-wiring.md`).
//!
//! 구조: `AiService`(앱 상태, command 가 부르는 동기 메서드) / `settings`·`store`(설정 문서) / `external`·`stt`·`embed`
//! (작업 코어 어댑터) / `models`(다운로드) / `events`(`models://progress`·`ai://analysis`) / `runner`(예약 루프).
//! command 래퍼는 `ai_commands.rs`.

#[cfg(test)]
mod cli_limit_tests;
mod cli_open;
#[cfg(test)]
mod cli_tests;
mod embed;
mod events;
mod external;
#[cfg(test)]
mod file_name_equiv_tests;
pub(crate) mod models;
mod persist_throttle;
mod runner;
pub(crate) mod service;
mod settings;
mod store;
#[allow(dead_code)]
// 프레임 스트립 셀 자르기 — 어댑터가 스트립 입력을 받을 때 쓴다 (업스트림 전환 예정)
mod strip;
mod stt;
mod sysinfo;
#[cfg(test)]
mod test_support;

pub use cli_open::ShellUrlOpener;
pub use embed::{LocalEmbedders, LocalQueryEmbedder};
pub use events::{EventSink, LateSink, TauriSink};
pub use models::ModelService;
pub use runner::AiRunner;
pub use service::{AiService, AiServiceDeps};
pub use settings::AiSettings;
pub use stt::system_tools;
pub use sysinfo::local_utc_offset_min;

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::error::{CommandError, ErrorCode};

/// AI 연결 계층의 실패 — command 경계에서 `CommandError` 로 바뀐다. 메시지에 키·경로는 없다.
#[derive(Debug)]
pub enum AiFailure {
    /// 외부 전송 동의 전
    ConsentRequired,
    /// 잘못된 입력 (필드 이름만)
    Invalid(&'static str),
    NotFound,
    /// 진행 중인 외부 AI 요청이 있다 (`ai_busy`)
    Busy,
    /// 구독 CLI 작업 실패 — 고정 사유 코드만 (`cli_not_found` | `cli_logout_failed`)
    Cli(&'static str),
    Db(mam_db::PoolError),
    Internal,
}

impl AiFailure {
    pub fn internal(cause: &dyn std::fmt::Display) -> Self {
        tracing::error!(%cause, "AI operation failed");
        Self::Internal
    }
}

impl From<mam_db::PoolError> for AiFailure {
    fn from(e: mam_db::PoolError) -> Self {
        Self::Db(e)
    }
}

impl From<AiFailure> for CommandError {
    fn from(e: AiFailure) -> Self {
        match e {
            AiFailure::ConsentRequired => Self::new(
                ErrorCode::ConsentRequired,
                "external AI consent is required",
            ),
            AiFailure::Invalid(what) => {
                Self::new(ErrorCode::InvalidInput, format!("invalid input: {what}"))
            }
            AiFailure::NotFound => Self::new(ErrorCode::NotFound, "not found"),
            AiFailure::Busy => Self::new(ErrorCode::AiBusy, "external AI request in progress"),
            AiFailure::Cli(code) => Self::new(ErrorCode::ProviderError, code),
            AiFailure::Db(e) => e.into(),
            AiFailure::Internal => Self::internal(),
        }
    }
}

/// 종료 신호 + 대기 (스레드 sleep 을 즉시 깨운다)
#[derive(Default)]
pub struct StopSignal {
    stopped: Mutex<bool>,
    cv: Condvar,
}

impl StopSignal {
    pub fn set(&self) {
        *self.stopped.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cv.notify_all();
    }

    /// `timeout` 동안 기다린다. 종료 신호가 오면 true.
    pub fn wait(&self, timeout: Duration) -> bool {
        let g = self.stopped.lock().unwrap_or_else(|p| p.into_inner());
        if *g {
            return true;
        }
        *self
            .cv
            .wait_timeout(g, timeout)
            .unwrap_or_else(|p| p.into_inner())
            .0
    }
}
