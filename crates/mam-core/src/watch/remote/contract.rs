//! 수집기 계약 타입 — 폴더 부착 정보, 기록기 trait, 시간 설정, 의존성 묶음. (`collector.rs` 가 다시 내보낸다)

use std::sync::{Arc, Mutex};
use std::path::PathBuf;
use std::time::Duration;

use super::RemoteSource;
use super::runner::RemoteConnector;
use crate::SharedClock;
use crate::watch::ChangeSink;

/// 한 폴더가 이 접속에 붙어 있다.
pub struct Attached {
    pub folder_id: i64,
    /// 폴더 루트 (확장 접두 제거한 문자열)
    pub root: String,
    pub sink: ChangeSink,
    /// 수집기가 이미 돌고 있는 중에 붙은 폴더 — 그 전의 줄은 못 봤으니 재스캔을 1회 요청한다
    pub needs_rescan: bool,
}

pub type Folders = Arc<Mutex<Vec<Attached>>>;

#[derive(Debug, thiserror::Error)]
#[error("cannot record remote changes: {0}")]
pub struct CommitError(pub String);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CommitOutcome {
    /// 폴더당 대기 행 상한에 막혀 새 경로를 받지 못한 폴더 (수집기가 재스캔 1회로 바꾼다)
    pub overflowed: Vec<i64>,
    /// 등록 해제 중이라 버린 보고 수
    pub dropped: usize,
}

/// 원격 피드가 대기열에 올릴 보고 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedItem {
    pub folder_id: i64,
    pub path: PathBuf,
    /// true = 하위 폴더 순회(`Subtree`), false = 경로 1건(`Path`)
    pub subtree: bool,
}

/// 보고 묶음과 커서를 **하나의 트랜잭션**으로 기록한다 (운영: `ingest::pending::remote`, 시험: 가짜).
pub trait BatchCommitter: Send + Sync {
    /// `items` 의 `watch_pending` UPSERT 와 `watch_cursors` 갱신이 함께 커밋되거나 함께 롤백된다. `origin` = 보고한 감지기 ID (진단용)
    fn commit_feed(&self, source_id: i64, items: &[FeedItem], cursor: &str, origin: &'static str) -> Result<CommitOutcome, CommitError>;

    /// `items` = (폴더 ID, 절대 경로) — 전부 `Path` 보고, 출처 `ddp_audit`
    fn commit(&self, source_id: i64, items: &[(i64, PathBuf)], cursor: &str) -> Result<CommitOutcome, CommitError> {
        let feed: Vec<FeedItem> = items.iter().map(|(f, p)| FeedItem { folder_id: *f, path: p.clone(), subtree: false }).collect();
        self.commit_feed(source_id, &feed, cursor, "ddp_audit")
    }
    /// 저장된 커서 (없으면 `None` — 처음 시작)
    fn load_cursor(&self, source_id: i64) -> Option<String>;
}

/// 저장된 접속 설정을 다시 읽는다 (설정 변경 후 재접속 때). 없으면 `None` = 삭제됨.
pub type SourceLoader = Arc<dyn Fn(i64) -> Option<RemoteSource> + Send + Sync>;

#[derive(Debug, Clone, Copy)]
pub struct Timings {
    pub reconnect_base: Duration,
    pub reconnect_max: Duration,
    /// 인증·호스트 키 오류 후 쉬는 시간 (서버 계정 잠금·fail2ban 방지)
    pub hard_retry: Duration,
    pub kick_gate_ms: i64,
    /// "정상 동작" 요약 로그 주기
    pub report_every_ms: i64,
    /// 시험용 — 접속 설정의 읽기 간격(`poll_sec`, 최소 1초) 대신 쓴다
    pub poll_override: Option<Duration>,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            reconnect_base: Duration::from_secs(15),
            reconnect_max: Duration::from_secs(60),
            hard_retry: Duration::from_secs(300),
            kick_gate_ms: 5_000,
            report_every_ms: 10 * 60_000,
            poll_override: None,
        }
    }
}

/// 수집기 스레드가 도는 동안 허브가 부르는 한 주기 단위 — 종류마다(`ddp_audit`·`ses_queue`) 구현이 다르다.
pub trait Feed: Send {
    /// 한 주기. 다음 주기까지 기다릴 시간을 돌려준다.
    fn step(&mut self) -> Duration;
    /// 접속 설정이 바뀌었다 — 접속을 닫고 새 설정으로 다시 붙는다.
    fn reload(&mut self, src: RemoteSource);
}

pub struct Deps {
    pub connector: Arc<dyn RemoteConnector>,
    pub committer: Arc<dyn BatchCommitter>,
    pub clock: SharedClock,
    pub timings: Timings,
    pub folders: Folders,
    pub loader: SourceLoader,
}
