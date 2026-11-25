//! 원격 명령 실행 추상 — SSH 구현(`ssh.rs`)과 시험용 가짜(`fake.rs`)가 같은 trait 를 구현한다.
//!
//! **읽기 전용**: 이 crate 가 원격에 보내는 명령은 `tail.rs` 의 `cmd::*` 두 가지(`wc -c` 목록, `tail -c`)와 `ses.rs` 의
//! `sqlite3 -readonly … SELECT` 뿐이다 — 쓰기·삭제·이동 명령은 없다.
//! 오류 문자열에는 비밀(비밀번호·키)이 들어가지 않는다 (`RunError` 는 접속 대상·종류만 담는다).

use std::fmt;

use super::RemoteSource;

/// 원격 호출 오류. `key()` 는 UI 번역 키 접미사(`remote.error.<key>`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// TCP·핸드셰이크 실패 (사유는 비밀을 담지 않는 짧은 글)
    #[error("cannot connect: {0}")]
    Connect(String),
    /// 계정·비밀번호·키가 거부됐다
    #[error("authentication rejected")]
    Auth,
    /// 개인 키를 읽을 수 없다 (형식 오류·암호가 걸린 키 — 지원하지 않는다)
    #[error("private key cannot be used")]
    BadKey,
    /// 저장된 지문과 다르다 — 장비가 바뀌었거나 중간자일 수 있다. 조용히 받아들이지 않는다.
    #[error("host key mismatch (expected {expected}, got {actual})")]
    HostKeyMismatch { expected: String, actual: String },
    /// 지문을 아직 확인하지 않았다 — 감시용 접속은 거부한다 (사용자가 지문을 확인해 저장해야 한다)
    #[error("host key is not confirmed yet (server offers {actual})")]
    HostKeyUnconfirmed { actual: String },
    #[error("remote command timed out")]
    Timeout,
    /// 로그 폴더가 없다 (종료 코드 9)
    #[error("log directory not found")]
    NoLogDir,
    /// 로그 폴더에 `audit-log.*` 가 없다 (감사 기능이 꺼져 있을 수 있다)
    #[error("no audit log files found")]
    NoAuditFiles,
    #[error("remote command failed: {0}")]
    Exec(String),
    /// 원격 호스트에 `sqlite3` 가 없다 (종료 코드 10, `ses_queue`)
    #[error("sqlite3 is not installed on the remote host")]
    NoSqlite,
    /// 이벤트 DB 를 읽을 수 없다 — 파일 없음·권한·잠김·테이블 없음 (종료 코드 11·12, `ses_queue`). 문구는 sqlite3 의 짧은 표준 오류(비밀 없음)
    #[error("event database cannot be read: {0}")]
    DbUnreadable(String),
    #[error("connection closed")]
    Closed,
}

impl RunError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::Auth => "auth",
            Self::BadKey => "bad_key",
            Self::HostKeyMismatch { .. } => "host_key_mismatch",
            Self::HostKeyUnconfirmed { .. } => "host_key_unconfirmed",
            Self::Timeout => "timeout",
            Self::NoLogDir => "no_log_dir",
            Self::NoAuditFiles => "no_audit_files",
            Self::Exec(_) => "exec",
            Self::NoSqlite => "no_sqlite",
            Self::DbUnreadable(_) => "db_unreadable",
            Self::Closed => "closed",
        }
    }

    /// 지문을 알려 줄 수 있는 오류인가 (접속 시험이 사용자에게 보여 준다)
    pub fn fingerprint(&self) -> Option<&str> {
        match self {
            Self::HostKeyMismatch { actual, .. } | Self::HostKeyUnconfirmed { actual } => {
                Some(actual)
            }
            _ => None,
        }
    }
}

/// 한 번 맺은 접속 위에서 읽기 명령을 돌린다. 버리면 접속이 닫힌다.
pub trait RemoteRunner: Send {
    /// 명령 하나를 돌리고 표준 출력을 돌려준다. 한 명령에 제한 시간이 있다.
    fn run(&mut self, cmd: &str) -> Result<Vec<u8>, RunError>;
}

/// 접속을 만든다. 운영은 `SshConnector`, 시험은 `FakeConnector`.
pub trait RemoteConnector: Send + Sync {
    /// 인증까지 마친 접속. `src.host_key` 가 없으면 `HostKeyUnconfirmed`, 다르면 `HostKeyMismatch` 로 **인증 전에** 거부한다.
    fn connect(&self, src: &RemoteSource) -> Result<Box<dyn RemoteRunner>, RunError>;

    /// 호스트 키 지문만 알아 온다 (인증하지 않는다 — 비밀을 보내지 않는다). 사용자가 확인한 뒤 `host_key` 로 저장한다.
    fn fingerprint(&self, host: &str, port: u16) -> Result<String, RunError>;
}

/// 호스트 키 판정 — SSH 구현과 가짜가 같은 규칙을 쓴다. 저장된 지문이 없으면 거부(`HostKeyUnconfirmed`), 다르면 거부(`HostKeyMismatch`).
/// 정확히 같아야 통과한다 (대소문자·공백 포함 — 조용한 허용 없음).
pub fn check_host_key(expected: Option<&str>, actual: &str) -> Result<(), RunError> {
    match expected {
        None => Err(RunError::HostKeyUnconfirmed {
            actual: actual.to_string(),
        }),
        Some(e) if e == actual => Ok(()),
        Some(e) => Err(RunError::HostKeyMismatch {
            expected: e.to_string(),
            actual: actual.to_string(),
        }),
    }
}

/// 시험·로그용 요약 — 접속 대상만 (비밀 없음).
pub struct Target<'a>(pub &'a RemoteSource);

impl fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}:{}", self.0.user, self.0.host, self.0.port)
    }
}
