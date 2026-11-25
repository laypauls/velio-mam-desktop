//! AI 분석 작업 코어 (P3-1·P3-3) - 외부 AI(P3-2)·로컬 STT(P3-4)·임베딩(P3-5)을 `JobKind::Ai` 실행기 하나로 묶는다.
//!
//! - 작업 단위 = **자산당 `ai` 작업 1개**. 처리할 단계는 `ai_task_state` 의 `queued` 행이다 (`jobs` 스키마 무변경)
//! - 단계 순서: stt(로컬) -> 외부 분석(describe/keywords/ocr/summary, 동의 필요, 요청 1회) -> embedding(로컬)
//!   단계마다 결과·상태를 즉시 저장하므로 일시정지·종료·실패 때에도 끝낸 단계는 보존되고, 다시 실행하면 queued 만 처리한다
//! - 실행기는 재시도 루프를 두지 않는다 - 백오프·`not_before` 는 대기열이 정하고, 429 `Retry-After` 는
//!   `JobError::retryable_after` 로 전달한다
//! - 외부 의존(공급자·STT·임베딩·설정)은 trait 로 주입한다 ([`types`], [`config::AiSettingsSource`])
//! - 등록(자동·백그라운드·일괄·재시도·동의 철회)은 [`AiScheduler`]

mod config;
mod executor;
mod external;
mod failure;
mod frames;
mod inflight;
mod plan;
mod sanitize;
mod schedule;
mod state;
mod steps;
mod types;

pub use config::{
    AiJobsConfig, AiRuntimeSettings, AiSettingsSource, Schedule, SttTimeout, local_hour,
};
pub use executor::{AiDeps, AiExecutor};
pub use failure::{FailureClass, classify};
pub use inflight::{InFlight, InFlightGuard};
pub use plan::{ACTIVE_SQL, EMBEDDING_BACKFILL_SQL, FAILED_SQL, QueueResult, UNANALYZED_SQL};
pub use schedule::AiScheduler;
pub use state::list_status;
pub use types::{
    EmbedMode, EmbedRequest, EmbeddingRunner, ExternalAnalyzer, ExternalRequest, ExternalResult,
    ExternalUsage, FrameCell, ImageInput, StepFailure, StepKind, StepState, StepStatus, SttRequest,
    SttRunner, Transfer,
};

use crate::ai_store::AiStoreError;
use crate::error::CoreError;
use crate::jobs::JobError;

#[derive(Debug, thiserror::Error)]
pub enum AiJobError {
    #[error(transparent)]
    Pool(#[from] mam_db::PoolError),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Store(#[from] AiStoreError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Queue(#[from] crate::jobs::JobsError),
}

impl From<AiJobError> for JobError {
    /// 일시적일 수 있는 DB·I/O 오류만 백오프 재시도 대상이다. 같은 입력으로 반복해도 결과가 같은
    /// 저장소 검증 오류(`Invalid`, `NotFound`)는 영구 실패(`fatal`)다 - 유료 외부 호출이 반복되지 않게 한다.
    fn from(e: AiJobError) -> Self {
        let msg = e.to_string();
        match e {
            AiJobError::Store(AiStoreError::Invalid(_) | AiStoreError::NotFound(_)) => {
                JobError::fatal(msg)
            }
            AiJobError::Core(ref c) if !matches!(c, CoreError::Io { .. }) => JobError::fatal(msg),
            _ => JobError::retryable(msg),
        }
    }
}

#[cfg(test)]
mod late_writes_tests;

#[cfg(test)]
mod tests {
    //! 분류: Business rule - 오류 재시도 분류
    use super::*;

    #[test]
    fn t_aij_31_store_validation_errors_are_fatal_and_transient_ones_retryable() {
        let invalid: JobError = AiJobError::Store(AiStoreError::Invalid("keyword too long")).into();
        let missing: JobError = AiJobError::Store(AiStoreError::NotFound(1)).into();
        let db: JobError = AiJobError::Sql(rusqlite::Error::QueryReturnedNoRows).into();
        let io: JobError = AiJobError::Core(CoreError::Io {
            path: "x".into(),
            source: std::io::Error::other("disk"),
        })
        .into();
        let logic: JobError = AiJobError::Core(CoreError::VolumeNotFound(1)).into();

        assert!(!invalid.retryable && !missing.retryable && !logic.retryable);
        assert!(db.retryable && io.retryable);
    }
}
