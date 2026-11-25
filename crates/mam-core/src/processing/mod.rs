//! 자산 처리 (P2-3) — 스캔이 등록한 자산에 **메타데이터 -> 썸네일(+영상 프레임 스트립) -> 전체 해시** 작업을 실행한다.
//!
//! - 작업은 `jobs` 테이블의 `meta`/`thumb`/`hash` 종류이며, 스캔이 자산을 만들거나 바꾸는 트랜잭션에서 함께 등록된다
//!   ([`derivatives::schedule`]). 실행기는 [`crate::ingest::Ingest::enable_processing`] 으로 꽂는다
//! - 결과 상태는 `asset_derivatives`(행 없음 = pending) 와 `media_assets.hash_state` 에 있다. 파일 존재만으로 판단하지 않는다
//! - 산출물(썸네일·스트립)은 MAMData `cache/` 에만 쓴다. 원본은 읽기 전용으로만 연다
//! - 처리 도중 자산이 바뀌었거나(`superseded`·크기·수정 시각 변화) 비활성이 되면 결과를 폐기한다
//! - 오프라인·파일 없음 자산은 건드리지 않고 작업을 끝낸다 (삭제가 아니다) — 볼륨이 돌아오면 [`backfill::backfill_jobs`] 가 다시 등록한다

mod backfill;
mod derivatives;
mod edge;
mod hash_job;
mod info;
mod merge;
mod meta_job;
mod source;
mod thumb_job;

use std::path::PathBuf;
use std::sync::Arc;

use mam_db::{DbPool, PoolError, Priority};
use mam_media::{MediaError, MediaTools};
use rusqlite::Connection;

use crate::SharedClock;
use crate::error::CoreError;
use crate::hash::ContentHasher;
use crate::jobs::{JobError, MAX_ATTEMPTS};

pub use backfill::{BackfillReport, backfill_jobs, retry_failed};
pub(crate) use derivatives::schedule_for_outcome;
#[cfg(test)]
pub(crate) use derivatives::set_state;
pub use derivatives::{DerivKind, DerivState, states_for};
pub use hash_job::HashExecutor;
pub use info::{MediaInfo, media_info};
pub use merge::{MergeReport, has_user_data, merge_after_hash};
pub use meta_job::MetaExecutor;
pub(crate) use source::{AssetSnapshot, Source, file_is_current, load_source};
pub use thumb_job::ThumbExecutor;

#[derive(Debug, thiserror::Error)]
pub enum ProcError {
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Core(#[from] CoreError),
}

impl From<ProcError> for JobError {
    /// DB·내부 오류는 일시적일 수 있으므로 백오프 재시도 대상이다.
    fn from(e: ProcError) -> Self {
        JobError::retryable(e.to_string())
    }
}

/// 실행기들이 공유하는 의존성.
pub struct ProcessingEnv {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub tools: Arc<MediaTools>,
    pub hasher: Arc<dyn ContentHasher + Send + Sync>,
    /// `MAMData/cache` — 썸네일은 `thumbnails/`, 프레임 스트립은 `previews/` 아래에 쓴다
    pub cache_root: PathBuf,
}

impl ProcessingEnv {
    pub fn new(
        pool: Arc<DbPool>,
        clock: SharedClock,
        tools: Arc<MediaTools>,
        hasher: Arc<dyn ContentHasher + Send + Sync>,
        cache_root: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            clock,
            tools,
            hasher,
            cache_root,
        })
    }

    /// 대량 작업 우선순위(`Low`)로 DB 쓰기 스레드에서 실행한다 — 사용자 편집(`High`)이 먼저 처리된다.
    pub(crate) fn write<T, F>(&self, f: F) -> Result<T, ProcError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, ProcError> + Send + 'static,
    {
        self.pool.write(Priority::Low, move |c| Ok(f(c)))?
    }

    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, ProcError>,
    ) -> Result<T, ProcError> {
        self.pool.read(|c| Ok(f(c)))?
    }
}

/// 미디어 처리 오류를 작업 결과로 바꾼다 (메타·썸네일 공통).
pub(crate) enum Verdict {
    /// 원본이 사라졌다 - 상태를 바꾸지 않고 작업을 끝낸다 (스캔이 missing 처리한다)
    Gone,
    /// 일시적인 I/O 오류·사이드카 시간 초과(부하) - 백오프 재시도
    Retry(String),
    /// 이 자산에는 불가능하다
    Unsupported(String),
    /// 앱 종료로 사이드카를 죽였다 - 실패가 아니다. 상태·시도 횟수를 바꾸지 않고 양보해 다음 실행에서 이어간다
    Interrupted,
    /// 실패로 기록한다 (도구 없음·재시도 소진·손상 등 - 같은 입력으로 반복해도 소용없다)
    Failed(String),
}

pub(crate) fn judge(e: &MediaError, attempts: i64) -> Verdict {
    match e {
        MediaError::Cancelled { .. } => Verdict::Interrupted,
        MediaError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound => {
            Verdict::Gone
        }
        // 공유 위반·네트워크 끊김·사이드카 시간 초과(PC 부하)는 잠시 뒤 성공할 수 있다. 마지막 시도에서는 실패로 기록한다
        MediaError::Io { .. } | MediaError::Timeout { .. } if attempts + 1 < MAX_ATTEMPTS => {
            Verdict::Retry(e.to_string())
        }
        MediaError::Unsupported(why) => Verdict::Unsupported((*why).to_string()),
        _ => Verdict::Failed(e.to_string()),
    }
}
