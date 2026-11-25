//! 단계별 처리 - stt / 외부 분석 / embedding. 각 단계는 끝나는 즉시 결과·상태·이력을 저장한다.

use std::path::PathBuf;

use mam_db::Clock;
use mam_media::{ThumbSize, thumbnail_path};
use rusqlite::{Connection, OptionalExtension};

use super::config::AiRuntimeSettings;
use super::executor::{AiExecutor, AssetInfo};
use super::failure::RetryNote;
use super::frames::Visual;
use super::types::{EmbedMode, EmbedRequest, StepFailure, StepKind, StepState, SttRequest};
use super::{AiJobError, state};
use crate::ai_store::{
    AiStoreError, RunRecord, SEMANTIC_ONLY_CODE, load_analysis, load_transcript, record_run,
    requeue_semantic_embedding, save_transcript,
};
use crate::jobs::Job;
use crate::processing::{Source, load_source};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Next,
    Yield,
    /// 필요한 파생물(썸네일·스트립)이 아직 만들어지는 중 - 시도 횟수·경고 없이 잠시 뒤로 미룬다
    Wait,
    /// 구독 한도 - 시도 횟수를 올리지 않고 이 시각(유닉스 ms)까지 작업을 미룬다
    WaitUntil(i64),
}

/// 성공한 단계를 done 으로 표시하고 성공 이력을 남긴다.
pub(super) fn finish_ok(
    c: &Connection,
    clock: &dyn Clock,
    asset_id: i64,
    kinds: &[StepKind],
    provider: &str,
    attempts: i64,
) -> Result<(), AiJobError> {
    let now = clock.now_ms();
    for k in kinds {
        state::set_step(c, now, asset_id, *k, StepState::Done, None, Some(attempts))?;
        record_run(
            c,
            clock,
            &RunRecord {
                asset_id,
                kind: k.as_str().into(),
                provider: provider.into(),
                ok: true,
                error_code: None,
                message: String::new(),
                attempts,
            },
        )?;
    }
    Ok(())
}

pub(super) fn skip_inactive(
    c: &Connection,
    now: i64,
    asset_id: i64,
    kinds: &[StepKind],
) -> Result<(), AiJobError> {
    for k in kinds {
        state::set_step(
            c,
            now,
            asset_id,
            *k,
            StepState::Skipped,
            Some("asset_inactive"),
            None,
        )?;
    }
    Ok(())
}

impl AiExecutor {
    /// 썸네일 파일 (큰 것 우선, 없으면 작은 것). 영상의 대표 프레임도 이 썸네일이다.
    pub(super) fn thumbs(&self, asset_id: i64) -> Vec<PathBuf> {
        [ThumbSize::Large, ThumbSize::Small]
            .into_iter()
            .map(|s| thumbnail_path(&self.cfg.cache_root, asset_id, s))
            .find(|p| p.is_file())
            .into_iter()
            .collect()
    }

    pub(super) fn step_stt(
        &self,
        job: &Job,
        asset_id: i64,
        info: &AssetInfo,
        st: &AiRuntimeSettings,
        stop: &dyn Fn() -> bool,
        note: &mut RetryNote,
    ) -> Result<Flow, AiJobError> {
        let kinds = [StepKind::Stt];
        if info.media_type == "image" {
            self.skip_all(asset_id, &kinds, "unsupported")?;
            return Ok(Flow::Next);
        }
        if !st.stt_enabled {
            self.skip_all(asset_id, &kinds, "disabled")?;
            return Ok(Flow::Next);
        }
        let path = match self.read(|c| Ok(load_source(c, asset_id)?))? {
            Source::Ready(_, p) => p,
            Source::Inactive => {
                self.skip_all(asset_id, &kinds, "asset_inactive")?;
                return Ok(Flow::Next);
            }
            // 볼륨 오프라인·파일 없음은 삭제가 아니다 - 재시도 대상으로 둔다
            Source::Unavailable => {
                let f = StepFailure::new("source_unavailable", "원본 파일에 접근할 수 없습니다");
                return self.apply_failure(job, asset_id, &kinds, "local", &f, None, note);
            }
        };
        self.mark_running(asset_id, &kinds)?;
        let req = SttRequest {
            asset_id,
            media_path: path,
            duration_ms: info.duration_ms,
            timeout: self.cfg.stt_timeout.for_duration(info.duration_ms),
        };
        match self.d.stt.transcribe(&req, stop) {
            Ok(rec) => {
                let attempts = job.attempts + 1;
                self.write(
                    move |c, clock| match save_transcript(c, clock, asset_id, &rec) {
                        Ok(_) => {
                            finish_ok(c, clock, asset_id, &kinds, "local", attempts)?;
                            // 전사가 바뀌었다 - 완료된 의미 벡터는 낡았다 (같은 작업이 이어서 의미 전용으로 다시 만든다)
                            requeue_semantic_embedding(c, clock.now_ms(), asset_id)?;
                            Ok(())
                        }
                        Err(AiStoreError::NotFound(_)) => {
                            skip_inactive(c, clock.now_ms(), asset_id, &kinds)
                        }
                        Err(e) => Err(e.into()),
                    },
                )?;
                Ok(Flow::Next)
            }
            Err(f) => self.apply_failure(job, asset_id, &kinds, "local", &f, None, note),
        }
    }

    pub(super) fn step_embedding(
        &self,
        job: &Job,
        asset_id: i64,
        info: &AssetInfo,
        st: &AiRuntimeSettings,
        note: &mut RetryNote,
    ) -> Result<Flow, AiJobError> {
        let kinds = [StepKind::Embedding];
        if !st.semantic_enabled {
            self.skip_all(asset_id, &kinds, "disabled")?;
            return Ok(Flow::Next);
        }
        let mode = self.embed_mode(asset_id)?;
        let images = if info.media_type == "audio" || mode == EmbedMode::SemanticOnly {
            Vec::new()
        } else {
            match self.visual_inputs(asset_id, &info.media_type, st.transfer.video_frames)? {
                Visual::Ready(t) => t,
                Visual::Waiting => return Ok(Flow::Wait),
                Visual::Missing => {
                    let f = StepFailure::new(
                        "thumbnail_missing",
                        "썸네일이 아직 만들어지지 않았습니다",
                    );
                    return self.apply_failure(job, asset_id, &kinds, "local", &f, None, note);
                }
            }
        };
        let text_parts = self.read(|c| {
            let mut parts = Vec::new();
            if let Some(a) = load_analysis(c, asset_id)? {
                parts.push(a.description);
                parts.push(a.keywords.join(", "));
                parts.push(a.ocr_text);
                parts.push(a.summary);
            }
            if let Some(t) = load_transcript(c, asset_id)? {
                parts.push(t.text);
            }
            Ok(parts)
        })?;
        self.mark_embedding_running(asset_id, mode)?;
        let req = EmbedRequest {
            asset_id,
            mode,
            images,
            text_parts,
        };
        match self.d.embedding.embed(&req) {
            Ok(()) => {
                let attempts = job.attempts + 1;
                self.write(move |c, clock| {
                    finish_ok(c, clock, asset_id, &kinds, "local", attempts)
                })?;
                Ok(Flow::Next)
            }
            Err(f) => {
                let flow = self.apply_failure(job, asset_id, &kinds, "local", &f, None, note)?;
                self.keep_semantic_marker(asset_id, mode)?;
                Ok(flow)
            }
        }
    }

    /// embedding 단계의 범위: `queued`/`running` 행의 `error_code` 가 의미 전용 표식이면 `SemanticOnly`.
    fn embed_mode(&self, asset_id: i64) -> Result<EmbedMode, AiJobError> {
        let code: Option<String> = self.read(|c| {
            Ok(c.query_row(
                "SELECT error_code FROM ai_task_state
                 WHERE asset_id = ?1 AND kind = 'embedding' AND state IN ('queued', 'running')",
                [asset_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
        })?;
        Ok(if code.as_deref() == Some(SEMANTIC_ONLY_CODE) {
            EmbedMode::SemanticOnly
        } else {
            EmbedMode::Full
        })
    }

    /// running 표시 - 의미 전용 표식은 비정상 종료 후 이어하기를 위해 유지한다.
    fn mark_embedding_running(&self, asset_id: i64, mode: EmbedMode) -> Result<(), AiJobError> {
        let code = (mode == EmbedMode::SemanticOnly).then_some(SEMANTIC_ONLY_CODE);
        self.write(move |c, clock| {
            state::set_step(
                c,
                clock.now_ms(),
                asset_id,
                StepKind::Embedding,
                StepState::Running,
                code,
                None,
            )?;
            Ok(())
        })
    }

    /// 재시도 가능 실패로 다시 대기(queued)가 된 의미 전용 단계는 표식을 되돌려 다음 시도도 의미 전용으로 한다
    /// (실패 코드는 `analysis_runs` 이력에 남아 있다).
    fn keep_semantic_marker(&self, asset_id: i64, mode: EmbedMode) -> Result<(), AiJobError> {
        if mode != EmbedMode::SemanticOnly {
            return Ok(());
        }
        self.write(move |c, _| {
            c.execute(
                "UPDATE ai_task_state SET error_code = ?2
                 WHERE asset_id = ?1 AND kind = 'embedding' AND state = 'queued'",
                rusqlite::params![asset_id, SEMANTIC_ONLY_CODE],
            )?;
            Ok(())
        })
    }
}
