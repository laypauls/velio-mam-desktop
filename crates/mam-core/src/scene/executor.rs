//! `scene` 작업 실행기 (설계 §4.3).

use std::sync::Arc;

use mam_db::scene_cuts::{SceneState, select_auto};
use mam_media::{MediaError, MediaKind, SceneDetectOptions, detect_scene_cuts};

use super::store::{Ctx, commit_cuts, commit_outcome, defer_job, load_ctx};
use super::threshold_for;
use crate::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome};
use crate::processing::{ProcessingEnv, Source, Verdict, judge, load_source};

/// 메타데이터가 아직 없을 때 다시 시도하기까지의 대기 (AI 실행기의 파생물 대기와 같은 방식)
const META_WAIT_MS: i64 = 60_000;

/// 행이 바뀐 자산 id 를 알리는 콜백 (프론트 `scene://updated` 이벤트용) — 커밋 뒤에만 부른다.
pub type SceneUpdated = Arc<dyn Fn(i64) + Send + Sync>;

pub struct SceneExecutor {
    env: Arc<ProcessingEnv>,
    on_updated: SceneUpdated,
}

impl SceneExecutor {
    pub fn new(env: Arc<ProcessingEnv>, on_updated: SceneUpdated) -> Self {
        Self { env, on_updated }
    }

    /// 실패·미지원 결과를 기록하고 알린다. 자산이 이미 사라졌으면 기록도 알림도 없다.
    fn record(
        &self,
        asset_id: i64,
        outcome: SceneState,
        error: Option<&'static str>,
    ) -> Result<JobOutcome, JobError> {
        let clock = Arc::clone(&self.env.clock);
        let wrote = self
            .env
            .write(move |c| commit_outcome(c, clock.now_ms(), asset_id, outcome, error))?;
        if wrote {
            (self.on_updated)(asset_id);
        }
        Ok(JobOutcome::Done)
    }

    fn run(&self, job: &Job, ctx: &JobContext<'_>, asset_id: i64) -> Result<JobOutcome, JobError> {
        let env = &self.env;
        let (snap, path) = match env.read(|c| Ok(load_source(c, asset_id)?))? {
            Source::Ready(snap, path) => (snap, path),
            // 비활성은 할 일이 없고, 오프라인·파일 없음은 삭제가 아니다 - 행은 pending 으로 남고 볼륨이 돌아오면 재등록된다
            Source::Inactive | Source::Unavailable => return Ok(JobOutcome::Done),
        };
        if snap.kind != MediaKind::Video {
            return self.record(asset_id, SceneState::Unsupported, None);
        }
        let Ctx {
            duration_ms,
            fps,
            meta_pending,
            sensitivity,
        } = env.read(|c| load_ctx(c, asset_id))?;
        if meta_pending {
            let (job_id, not_before) =
                (job.job_id, env.clock.now_ms().saturating_add(META_WAIT_MS));
            env.write(move |c| defer_job(c, job_id, not_before))?;
            return Ok(JobOutcome::Yielded);
        }
        if ctx.should_stop() {
            return Ok(JobOutcome::Yielded);
        }
        // DB 의 크기·수정 시각이 낡았으면 스캔이 곧 갱신하고 새 작업을 등록한다 - 지금 결과는 쓰지 않는다
        if !crate::processing::file_is_current(&path, &snap) {
            return Ok(JobOutcome::Done);
        }
        let opts = SceneDetectOptions::new(threshold_for(sensitivity), duration_ms);
        match detect_scene_cuts(&env.tools, &path, &opts, &|| ctx.should_stop()) {
            Ok(raw) => {
                let raw: Vec<(i64, f32)> = raw.iter().map(|c| (c.ms, c.score)).collect();
                let auto = select_auto(&raw, duration_ms);
                let clock = Arc::clone(&env.clock);
                let stored = env.write(move |c| {
                    commit_cuts(c, clock.now_ms(), asset_id, &snap, &auto, sensitivity, fps)
                })?;
                if stored {
                    (self.on_updated)(asset_id);
                }
                Ok(JobOutcome::Done)
            }
            Err(e) => match judge(&e, job.attempts) {
                Verdict::Gone => Ok(JobOutcome::Done),
                Verdict::Interrupted => Ok(JobOutcome::Yielded),
                Verdict::Retry(msg) => Err(JobError::retryable(msg)),
                Verdict::Unsupported(_) => self.record(asset_id, SceneState::Unsupported, None),
                Verdict::Failed(_) => {
                    self.record(asset_id, SceneState::Failed, Some(error_code(&e)))
                }
            },
        }
    }
}

/// 저장하는 고정 오류 코드 (경로·stderr 는 저장하지 않는다 - 로그에만 남는다)
fn error_code(e: &MediaError) -> &'static str {
    match e {
        MediaError::ToolNotFound { .. } => "tool_not_found",
        MediaError::Timeout { .. } => "timeout",
        MediaError::ToolFailed { .. } => "tool_failed",
        MediaError::OutputTooLarge { .. } => "output_too_large",
        MediaError::Io { .. } => "io",
        _ => "other",
    }
}

impl JobExecutor for SceneExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let asset_id = job
            .asset_id
            .ok_or_else(|| JobError::fatal("scene job without asset"))?;
        self.run(job, ctx, asset_id)
    }
}
