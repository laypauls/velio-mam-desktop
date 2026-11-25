//! 구독 인증 — 사용자가 이 PC 에 설치한 Claude Code CLI 를 비대화 모드로 대리 실행한다 (claude 전용).
//!
//! 원칙: 토큰·자격 증명 파일을 읽거나 저장하지 않는다(인증은 CLI 책임), CLI 의 내장 인증 방식을 제한하지 않는다
//! (환경을 그대로 상속하고 `apiKeySource` 만 감지), 도구는 전부 끄고 빈 임시 폴더에서 실행한다,
//! 외부 전송은 MAM 파생 이미지·정제 메타뿐이다(동의 게이트 필수). 계획·실측: `docs/requests/2026-11/20261107-135215-ai-subscription-auth.md`.
//!
//! 구성: `process`/`os_runner`(실행기 trait + std::process 구현) · `exec`(줄 스트리밍·제한 시간·PID 추적) ·
//! `resolve`(실행 파일 탐색) · `status`(버전·계정) · `args`/`stream`(요청 조립·응답 해석) · `workdir` ·
//! `host`(탐지·직렬화·로그아웃) · `client`(`AiClient`) · `login`/`login_url`(브라우저 로그인 연동) · `fake`(시험용).

mod args;
mod client;
mod exec;
pub mod fake;
mod host;
mod login;
mod login_url;
mod os_runner;
mod process;
mod resolve;
mod status;
mod stream;
mod workdir;

pub use args::DEFAULT_CLI_MODEL;
pub use client::{CliClient, REQUEST_CAP};
pub use exec::{Exec, Flow, RunEnd};
pub use host::{CliHost, LogoutError};
pub use login::{CliLogin, LoginState, LoginTimings, UrlOpener};
pub use os_runner::OsRunner;
pub use process::{CliCommand, CliProcess, CliRunner, ProcEvent};
pub use resolve::{CliEnv, SystemEnv, resolve_claude};
pub use status::{CliAccount, CliStatus, MIN_VERSION, Version, parse_auth_status, parse_version};
pub use stream::OFFLINE_AFTER_RETRIES;
pub use workdir::{WorkDir, default_work_root};

#[cfg(test)]
mod tests;
