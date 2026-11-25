//! 원격 스토리지 접속 프레임워크 — 이 PC 에서 OS 변경 이벤트가 오지 않는 스토리지(DDP 등)의 **감사 로그를 SSH 로 이어 읽어**
//! 바뀐 경로를 감시 계층(`ChangeSink`)에 알린다 (계획 §4, step 5). watch_folder `internal/ddpaudit` 의 동작 규칙·시험을 옮겼다.
//!
//! ```text
//! parse(감사 줄) ◄─ tail(파일별 오프셋·1 MiB 상한) ◄─ runner(SSH / 가짜) ◄─ collector(접속 1개 = 스레드 1개, 폴더 여럿이 공유)
//!                                                          mapper(볼륨 경로 -> PC 경로) ─► 폴더 루트 아래만 보고 ─► committer(대기열+커서 1 트랜잭션)
//! ```
//!
//! 원칙: **원격에는 읽기 명령만** 보낸다 · 접속은 사용자가 만든 `remote_sources` 행에만 · 비밀은 DPAPI blob 으로만 저장하고
//! 로그·에러·IPC 에 내지 않는다 · 호스트 키는 사용자가 확인한 지문과 **정확히** 같아야 한다 · 감사 로그는 주기 재스캔을 대신하지 않는다
//! (놓쳤을 수 있으면 `Rescan`).

pub mod collector;
mod contract;
pub mod fake;
pub mod hub;
pub mod mapper;
pub mod parse;
pub mod fake_ses;
pub mod runner;
pub mod ses;
pub mod ses_collector;
pub mod ssh;
pub mod stats;
mod stop;
pub mod store;
pub mod tail;

use crate::secret::Secret;

pub use collector::{BatchCommitter, CommitOutcome, Timings};
pub use hub::RemoteHub;
pub use mapper::{Mapper, Mapping, map_reaches, within_dir};
pub use runner::{RemoteConnector, RemoteRunner, RunError};
pub use ssh::SshConnector;
pub use store::{NewSource, SourceView, StoreError};

/// 기본 로그 폴더 (AVFS)
pub const DEFAULT_LOG_DIR: &str = "/tmp/avfs-dbase/audit";
pub const DEFAULT_POLL_SEC: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    DdpAudit,
    /// storage-event-service 큐 (`fanotify_feed`) — 리눅스 스토리지 호스트의 `events` SQLite 를 읽기 전용으로 이어 읽는다
    SesQueue,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DdpAudit => "ddp_audit",
            Self::SesQueue => "ses_queue",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ddp_audit" => Some(Self::DdpAudit),
            "ses_queue" => Some(Self::SesQueue),
            _ => None,
        }
    }
}

/// 인증 수단. 내용은 [`Secret`] 이라 `Debug` 에 찍히지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    Password(Secret),
    /// 개인 키 PEM 본문 (암호가 걸린 키는 지원하지 않는다)
    Key(Secret),
}

impl Auth {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Password(_) => "password",
            Self::Key(_) => "key",
        }
    }

    pub fn secret(&self) -> &Secret {
        match self {
            Self::Password(s) | Self::Key(s) => s,
        }
    }
}

/// 접속 하나의 복호된 설정 — 접속할 때만 메모리에 만든다. `Serialize` 를 구현하지 않는다.
#[derive(Debug, Clone)]
pub struct RemoteSource {
    pub source_id: i64,
    pub kind: SourceKind,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// 사용자가 확인한 호스트 키 지문. `None` 이면 감시용 접속을 거부한다
    pub host_key: Option<String>,
    pub log_dir: Option<String>,
    pub db_path: Option<String>,
    pub poll_sec: u32,
    pub path_map: Vec<Mapping>,
}

impl RemoteSource {
    pub fn log_dir(&self) -> &str {
        self.log_dir.as_deref().unwrap_or(DEFAULT_LOG_DIR)
    }
}
