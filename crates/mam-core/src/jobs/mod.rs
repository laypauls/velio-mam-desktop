//! 작업 대기열 (P2-2, D7) — `jobs` 테이블에 영속화된 우선순위 큐 + 종류별 동시성 + 일시정지/재개 + 실패 백오프.
//!
//! - 우선순위(작을수록 먼저): 스캔 < 메타·썸네일 < 색인 < 해시 < AI < 장면 검출. 사용자 편집은 대기열이 아니라 DB 쓰기 우선순위(`High`)가 담당한다
//! - 같은 (kind, asset) / (kind, folder) 의 **대기 중** 작업은 하나만 둔다 (멱등 등록)
//! - 앱 재시작 시 `running` -> `pending` ([`JobQueue::recover`]) — 이어하기는 각 실행기가 자기 체크포인트로 한다
//! - 실패는 지수 백오프로 재시도하고 [`MAX_ATTEMPTS`] 회 뒤 `failed`. 수동 재시도는 [`JobQueue::retry_failed`]
//! - 실행기는 [`JobExecutor`] 로 꽂는다 (메타·썸네일·해시 실행기는 P2-3). 실행기가 없는 종류는 claim 하지 않고 대기로 둔다
//!
//! 스레드 모델: 디스패처 스레드 1개 + 작업당 실행 스레드(종류별 동시성 상한 이내). 일시정지는 새 작업의 시작을
//! 막고, 실행 중인 작업에는 [`JobContext::should_stop`] 로 알린다 (체크포인트에서 `Yielded` 반환).

mod queue;
mod runner;

use serde::{Deserialize, Serialize};

pub use queue::{JobQueue, NewJob, backoff_ms};
pub use runner::{ExecutorRegistry, JobContext, JobExecutor, JobRunner};

/// 실패 작업을 포기하기까지의 시도 횟수 (D7).
pub const MAX_ATTEMPTS: i64 = 5;
/// 동시에 도는 스캔의 전체 상한 기본값 (결정 E). 설정 `maxConcurrentScans` 의 기본값과 같다
pub const DEFAULT_SCAN_CAP: usize = 3;
/// 설정 `maxConcurrentScans` 의 허용 범위 — 볼륨별로 하나씩이라 볼륨 수 이상은 의미가 없고, 쓰기 스레드는 하나뿐이다
pub const SCAN_CAP_RANGE: std::ops::RangeInclusive<usize> = 1..=8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Scan,
    Meta,
    Thumb,
    Hash,
    Index,
    Ai,
    /// 동영상 장면전환점 검출 (영상 전체 디코딩 - 가장 무거운 작업이라 우선순위가 가장 낮다)
    Scene,
    /// 플러그인 provider 실행 (자산 1개 = 작업 1개, 그 자산의 대기 `plugin_tasks` 를 순서대로 처리)
    Plugin,
}

impl JobKind {
    /// 종류 수 - 종류별 배열의 길이 (`idx()` 가 인덱스)
    pub const COUNT: usize = 8;

    pub const ALL: [JobKind; Self::COUNT] = [
        Self::Scan,
        Self::Meta,
        Self::Thumb,
        Self::Hash,
        Self::Index,
        Self::Ai,
        Self::Scene,
        Self::Plugin,
    ];

    /// `jobs.kind` CHECK 값 = 프론트 계약의 `JobKind`
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Meta => "meta",
            Self::Thumb => "thumb",
            Self::Hash => "hash",
            Self::Index => "index",
            Self::Ai => "ai",
            Self::Scene => "scene",
            Self::Plugin => "plugin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub(crate) fn idx(self) -> usize {
        self as usize
    }

    /// 종류별 기본 우선순위 (작을수록 먼저)
    pub fn default_priority(self) -> i64 {
        match self {
            Self::Scan => 10,
            Self::Meta | Self::Thumb => 20,
            Self::Index => 25,
            Self::Hash => 30,
            Self::Ai => 50,
            // 백필·자동 등록 (사용자 요청은 `mam_db::scene_cuts::SCENE_PRIORITY_USER` 로 등록)
            Self::Scene => 60,
            // 변경 피드·전체 실행 (메뉴 명령·내보내기는 대기열 밖에서 직접 실행되므로 더 우선한다)
            Self::Plugin => 70,
        }
    }

    /// 종류별 기본 동시성 상한 (해시 1, 썸네일 2 — D7). 스캔은 **볼륨별 1개 + 전체 상한**(결정 E)이라
    /// 이 값은 전체 상한의 기본값이다 — 같은 볼륨의 스캔은 항상 하나씩이다 (설정 `maxConcurrentScans` 로 바꾼다)
    pub fn default_concurrency(self) -> usize {
        match self {
            Self::Scan => DEFAULT_SCAN_CAP,
            Self::Thumb | Self::Meta => 2,
            _ => 1,
        }
    }
}

/// 실행기에 전달되는 작업 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub job_id: i64,
    pub kind: JobKind,
    pub asset_id: Option<i64>,
    pub folder_id: Option<i64>,
    pub priority: i64,
    pub attempts: i64,
}

// 일시정지 비트마스크(`AtomicU8`)가 종류를 모두 담을 수 있어야 한다
const _: () = assert!(JobKind::COUNT <= 8);

/// 실행기가 정상 반환한 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobOutcome {
    /// 완료 — 행을 지운다
    Done,
    /// 일시정지·종료로 중단 — 시도 횟수를 올리지 않고 대기로 되돌린다 (체크포인트에서 이어간다)
    Yielded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobError {
    pub message: String,
    /// false 면 재시도하지 않고 바로 `failed`
    pub retryable: bool,
    /// 공급자가 요청한 최소 대기 (429 `Retry-After`). 다음 시도 시각은 `max(백오프, retry_after)`.
    pub retry_after: Option<std::time::Duration>,
}

impl JobError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            retry_after: None,
        }
    }
    /// 재시도 가능 + 최소 `after` 뒤에 다시 시도 (백오프가 더 길면 백오프를 따른다).
    pub fn retryable_after(message: impl Into<String>, after: std::time::Duration) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            retry_after: Some(after),
        }
    }
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
            retry_after: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct JobCounts {
    pub pending: u64,
    pub running: u64,
    pub failed: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum JobsError {
    #[error(transparent)]
    Pool(#[from] mam_db::PoolError),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}
