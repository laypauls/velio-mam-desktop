//! `ai` 작업 실행기 - 자산 1개의 queued 단계를 stt -> 외부 분석 -> embedding 순서로 처리한다.

use std::sync::Arc;

use mam_db::{Clock, DbError, DbPool, Priority};
use rusqlite::{Connection, OptionalExtension};

use super::config::{AiJobsConfig, AiSettingsSource};
use super::failure::{FailureClass, RetryNote, classify};
use super::inflight::InFlight;
use super::steps::Flow;
use super::types::{
    EmbeddingRunner, ExternalAnalyzer, StepFailure, StepKind, StepState, SttRunner,
};
use super::{AiJobError, state};
use crate::SharedClock;
use crate::ai_store::{RunRecord, UsageEntry, record_run, record_usage};
use crate::jobs::{
    ExecutorRegistry, Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, MAX_ATTEMPTS,
};

/// 필요한 파생물(썸네일·프레임 스트립)이 아직 없을 때 작업을 미루는 시간.
const NOT_READY_WAIT_MS: i64 = 30_000;
/// 한도 초기화 시각을 모를 때 구독 한도 대기 시간(초).
const PAUSE_FALLBACK_SECS: u64 = 3600;

/// 실행기가 쓰는 외부 의존성.
pub struct AiDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub settings: Arc<dyn AiSettingsSource>,
    pub external: Arc<dyn ExternalAnalyzer>,
    pub stt: Arc<dyn SttRunner>,
    pub embedding: Arc<dyn EmbeddingRunner>,
}

pub struct AiExecutor {
    pub(super) d: AiDeps,
    pub(super) cfg: AiJobsConfig,
    pub(super) in_flight: InFlight,
}

pub(super) struct AssetInfo {
    pub media_type: String,
    pub duration_ms: Option<u64>,
}

impl AiExecutor {
    pub fn new(deps: AiDeps, cfg: AiJobsConfig) -> Self {
        Self {
            d: deps,
            cfg,
            in_flight: InFlight::new(),
        }
    }

    /// 종료 경로와 공유할 진행 중 외부 요청 계수기로 바꾼다 (`register` 로 소비하기 전에 호출).
    pub fn with_in_flight(mut self, in_flight: InFlight) -> Self {
        self.in_flight = in_flight;
        self
    }

    /// `JobKind::Ai` 실행기로 등록한다 (`JobQueue` 는 `cfg.queue_caps()` 로 만든다).
    pub fn register(self, registry: &mut ExecutorRegistry) {
        registry.register(JobKind::Ai, Arc::new(self));
    }

    pub(super) fn now(&self) -> i64 {
        self.d.clock.now_ms()
    }

    /// 대량 작업 우선순위(`Low`)로 DB 쓰기 스레드에서 실행한다 - 사용자 편집(`High`)이 먼저 처리된다.
    pub(super) fn write<T, F>(&self, f: F) -> Result<T, AiJobError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection, &dyn Clock) -> Result<T, AiJobError> + Send + 'static,
    {
        let clock = Arc::clone(&self.d.clock);
        self.d
            .pool
            .write(Priority::Low, move |c| Ok::<_, DbError>(f(c, &*clock)))?
    }

    pub(super) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, AiJobError>,
    ) -> Result<T, AiJobError> {
        self.d.pool.read(|c| Ok::<_, DbError>(f(c)))?
    }

    fn asset_info(&self, asset_id: i64) -> Result<Option<AssetInfo>, AiJobError> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT a.media_type, m.duration_ms FROM media_assets a
                 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
                 WHERE a.asset_id = ?1 AND a.status = 'active'",
                [asset_id],
                |r| {
                    Ok(AssetInfo {
                        media_type: r.get(0)?,
                        duration_ms: r
                            .get::<_, Option<i64>>(1)?
                            .and_then(|d| u64::try_from(d).ok()),
                    })
                },
            )
            .optional()?)
        })
    }

    /// 진행 중 외부 요청 계수기 (종료 경로가 요청 완료를 기다릴 때 쓴다).
    pub fn in_flight(&self) -> InFlight {
        self.in_flight.clone()
    }

    /// 중단 지점마다 호출 - `stop()` 이면 아직 시작하지 않은 단계는 queued 그대로 두고 양보한다.
    pub fn run(&self, job: &Job, stop: &dyn Fn() -> bool) -> Result<JobOutcome, JobError> {
        let asset_id = job
            .asset_id
            .ok_or_else(|| JobError::fatal("ai job without asset"))?;
        let steps: Vec<StepKind> = self
            .read(|c| Ok(state::pending_steps(c, asset_id)?))?
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        if steps.is_empty() {
            return Ok(JobOutcome::Done);
        }
        let Some(info) = self.asset_info(asset_id)? else {
            // 자산이 없거나 활성이 아니다 - 할 일이 없다 (실패 아님)
            self.skip_all(asset_id, &steps, "asset_inactive")?;
            return Ok(JobOutcome::Done);
        };
        let mut note = RetryNote::default();
        let flow = self.run_steps(job, asset_id, &info, &steps, stop, &mut note)?;
        if flow == Flow::Wait && note.is_empty() {
            // 파생물이 아직 만들어지는 중 - 시도·경고 없이 잠시 뒤에 다시 (재시도 실패가 함께 있으면 그쪽이 우선)
            tracing::debug!(asset_id, "ai job waits for thumbnail/strip derivative");
            self.defer(job)?;
            return Ok(JobOutcome::Yielded);
        }
        if let Flow::WaitUntil(t) = flow
            && note.is_empty()
        {
            // 구독 한도 - 시도 횟수를 올리지 않고 한도가 풀릴 때까지 미룬다 (`defer` 와 같은 방식, 시각만 다르다)
            tracing::info!(
                asset_id,
                "ai job deferred until the subscription limit resets"
            );
            self.defer_until(job, t)?;
            return Ok(JobOutcome::Yielded);
        }
        if flow == Flow::Yield {
            return Ok(JobOutcome::Yielded);
        }
        if note.is_empty() {
            Ok(JobOutcome::Done)
        } else {
            Err(note.into_job_error())
        }
    }

    /// 실행 중인 작업 행의 `not_before` 를 미룬다. 이어서 `Yielded` 로 끝나면 대기열은 시도 횟수를 올리지 않고
    /// 행을 `pending` 으로 되돌리며 `not_before` 는 그대로 둔다 (`jobs::JobQueue::finish`).
    fn defer(&self, job: &Job) -> Result<(), AiJobError> {
        self.defer_until(job, self.now().saturating_add(NOT_READY_WAIT_MS))
    }

    fn defer_until(&self, job: &Job, not_before: i64) -> Result<(), AiJobError> {
        let job_id = job.job_id;
        self.write(move |c, _| {
            c.execute(
                "UPDATE jobs SET not_before = ?2 WHERE job_id = ?1",
                rusqlite::params![job_id, not_before],
            )?;
            Ok(())
        })
    }

    fn run_steps(
        &self,
        job: &Job,
        asset_id: i64,
        info: &AssetInfo,
        steps: &[StepKind],
        stop: &dyn Fn() -> bool,
        note: &mut RetryNote,
    ) -> Result<Flow, AiJobError> {
        let settings = self.d.settings.snapshot();
        if steps.contains(&StepKind::Stt) {
            if stop() {
                return Ok(Flow::Yield);
            }
            if self.step_stt(job, asset_id, info, &settings, stop, note)? == Flow::Yield {
                return Ok(Flow::Yield);
            }
        }
        let ext: Vec<StepKind> = steps.iter().copied().filter(|k| k.is_external()).collect();
        if !ext.is_empty() {
            if stop() {
                return Ok(Flow::Yield);
            }
            let settings = self.d.settings.snapshot(); // STT 가 길었을 수 있다 - 동의 상태를 다시 읽는다
            let flow = self.step_external(job, asset_id, info, &ext, &settings, stop, note)?;
            if flow != Flow::Next {
                return Ok(flow);
            }
        }
        // 외부 분석이 방금 끝났다면 embedding 이 다시 queued 가 됐을 수 있다 - 대기 목록을 다시 읽는다
        let embed_pending = self
            .read(|c| Ok(state::pending_steps(c, asset_id)?))?
            .iter()
            .any(|(k, _)| *k == StepKind::Embedding);
        if embed_pending {
            if stop() {
                return Ok(Flow::Yield);
            }
            let settings = self.d.settings.snapshot();
            return self.step_embedding(job, asset_id, info, &settings, note);
        }
        Ok(Flow::Next)
    }

    pub(super) fn skip_all(
        &self,
        asset_id: i64,
        kinds: &[StepKind],
        code: &'static str,
    ) -> Result<(), AiJobError> {
        let kinds = kinds.to_vec();
        self.write(move |c, clock| {
            let now = clock.now_ms();
            for k in kinds {
                state::set_step(c, now, asset_id, k, StepState::Skipped, Some(code), None)?;
            }
            Ok(())
        })
    }

    pub(super) fn mark_running(&self, asset_id: i64, kinds: &[StepKind]) -> Result<(), AiJobError> {
        let kinds = kinds.to_vec();
        self.write(move |c, clock| {
            let now = clock.now_ms();
            for k in kinds {
                state::set_step(c, now, asset_id, k, StepState::Running, None, None)?;
            }
            Ok(())
        })
    }

    /// 실패를 분류해 단계 상태·이력(·사용량)에 반영한다. 한 트랜잭션 단위로 기록한다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_failure(
        &self,
        job: &Job,
        asset_id: i64,
        kinds: &[StepKind],
        provider: &str,
        f: &StepFailure,
        usage: Option<UsageEntry>,
        note: &mut RetryNote,
    ) -> Result<Flow, AiJobError> {
        let class = classify(&f.code);
        let attempts = job.attempts + 1;
        let (new_state, record) = match class {
            FailureClass::Cancel | FailureClass::Pause => (StepState::Queued, false),
            FailureClass::Skip => (StepState::Skipped, false),
            FailureClass::Retry if attempts >= MAX_ATTEMPTS => (StepState::Failed, true),
            FailureClass::Retry => (StepState::Queued, true),
            FailureClass::Permanent => (StepState::Failed, true),
        };
        if class == FailureClass::Retry {
            note.add(&f.code, f.retry_after_secs);
        }
        let retry_after = f.retry_after_secs;
        let (kinds, provider, f) = (kinds.to_vec(), provider.to_string(), f.clone());
        self.write(move |c, clock| {
            let now = clock.now_ms();
            let tx = c.transaction()?;
            for k in kinds {
                let counted = (!matches!(class, FailureClass::Cancel | FailureClass::Pause))
                    .then_some(attempts);
                state::set_step(&tx, now, asset_id, k, new_state, Some(&f.code), counted)?;
                if record {
                    record_run(
                        &tx,
                        clock,
                        &RunRecord {
                            asset_id,
                            kind: k.as_str().into(),
                            provider: provider.clone(),
                            ok: false,
                            error_code: Some(f.code.clone()),
                            message: f.message.clone(),
                            attempts,
                        },
                    )?;
                }
            }
            if let Some(u) = &usage {
                record_usage(&tx, u)?;
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(match class {
            FailureClass::Cancel => Flow::Yield,
            FailureClass::Pause => Flow::WaitUntil(
                self.now().saturating_add(
                    i64::try_from(retry_after.unwrap_or(PAUSE_FALLBACK_SECS))
                        .unwrap_or(i64::MAX / 2000)
                        .saturating_mul(1000),
                ),
            ),
            _ => Flow::Next,
        })
    }
}

impl JobExecutor for AiExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        self.run(job, &|| ctx.should_stop())
    }
}
