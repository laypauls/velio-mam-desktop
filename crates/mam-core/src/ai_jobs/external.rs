//! 외부 AI 분석 단계 - 동의 확인, 썸네일만 전송, 결과·사용량·이력 저장.

use super::AiJobError;
use super::config::AiRuntimeSettings;
use super::executor::{AiExecutor, AssetInfo};
use super::failure::{FailureClass, RetryNote, classify, request_was_sent};
use super::frames::Visual;
use super::sanitize::sanitize;
use super::steps::{Flow, finish_ok, skip_inactive};
use super::types::{ExternalRequest, ExternalResult, StepFailure, StepKind};
use crate::ai_store::{AiStoreError, AnalysisFields, NewAnalysis, UsageEntry, record_usage};
use crate::jobs::Job;
use rusqlite::OptionalExtension;

impl AiExecutor {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn step_external(
        &self,
        job: &Job,
        asset_id: i64,
        info: &AssetInfo,
        kinds: &[StepKind],
        st: &AiRuntimeSettings,
        stop: &dyn Fn() -> bool,
        note: &mut RetryNote,
    ) -> Result<Flow, AiJobError> {
        // 동의 전에는 어떤 것도 전송하지 않는다 - 로컬 단계는 계속 실행된다
        if !st.consent_given {
            self.skip_all(asset_id, kinds, "consent_required")?;
            return Ok(Flow::Next);
        }
        if info.media_type == "audio" {
            self.skip_all(asset_id, kinds, "unsupported")?;
            return Ok(Flow::Next);
        }
        // 구독 한도 중에는 요청도 프로세스도 만들지 않는다 - 시도 횟수를 올리지 않고 한도가 풀릴 때까지 작업을 미룬다
        if let Some(until) = self.d.external.paused_until_ms()
            && until > self.now()
        {
            return Ok(Flow::WaitUntil(until));
        }
        let provider = self.d.external.provider();
        let configured_mode = self.d.external.auth_mode();
        let images_in =
            match self.visual_inputs(asset_id, &info.media_type, st.transfer.video_frames)? {
                Visual::Ready(t) => t,
                Visual::Waiting => return Ok(Flow::Wait),
                Visual::Missing => {
                    let f = StepFailure::new(
                        "thumbnail_missing",
                        "썸네일이 아직 만들어지지 않았습니다",
                    );
                    return self.apply_failure(job, asset_id, kinds, &provider, &f, None, note);
                }
            };
        // 요청을 잡아 둔 뒤 정지 여부를 확인한다 - 종료 경로는 정지 신호를 먼저 세운 뒤 계수를 읽으므로, 둘 중 하나는 반드시 서로를 본다.
        // 정지 중이면 아직 보내지 않았으니 단계는 queued 그대로 두고 양보한다 (시도·사용량·실패 없음).
        // 가드는 응답·사용량 저장(또는 오류 처리)이 끝나는 이 함수 끝까지 유지한다.
        let _in_flight = self.in_flight.begin();
        if stop() {
            return Ok(Flow::Yield);
        }
        self.mark_running(asset_id, kinds)?;
        let images = u32::try_from(images_in.len()).unwrap_or(u32::MAX);
        let req = ExternalRequest {
            asset_id,
            media_type: info.media_type.clone(),
            kinds: kinds.to_vec(),
            language: st.language.clone(),
            transfer: st.transfer,
            images: images_in,
        };
        match self.d.external.analyze(&req) {
            Ok(res) => {
                self.store_external(job, asset_id, kinds, &st.language, res, configured_mode)
            }
            Err(f) => {
                // 실제로 요청이 나간 실패만 사용량에 남긴다 (건너뜀·중단은 요청 없음)
                let usage = (matches!(
                    classify(&f.code),
                    FailureClass::Retry | FailureClass::Permanent | FailureClass::Pause
                ) && request_was_sent(&f.code))
                .then(|| UsageEntry {
                    provider: provider.clone(),
                    model: String::new(),
                    kind: join_kinds(kinds),
                    asset_id: Some(asset_id),
                    input_tokens: 0,
                    output_tokens: 0,
                    request_bytes: 0,
                    images,
                    ok: false,
                    error_code: Some(f.code.clone()),
                    at: self.now(),
                    auth_mode: f.billed_auth_mode.clone().or(configured_mode.clone()),
                });
                self.apply_failure(job, asset_id, kinds, &provider, &f, usage, note)
            }
        }
    }

    /// 응답 저장. 순서가 중요하다:
    /// 1. 요청은 이미 나갔고 비용이 발생했으므로 **사용량을 먼저 자체 트랜잭션으로 커밋**한다 (이후 저장이 실패해도 남는다)
    /// 2. 응답을 정제하고(`sanitize`) 분석 저장 + 단계 완료 + 임베딩 재대기를 **한 트랜잭션**으로 커밋한다.
    ///    저장은 됐는데 완료 표시만 실패하는 틈이 없으므로, 재시도가 같은 요청을 다시 보내지 않는다.
    /// 3. 정제 후에도 저장소가 거부하면(`Invalid`) 같은 입력으로 반복해도 소용없다 - 단계를 영구 실패로 기록한다.
    fn store_external(
        &self,
        job: &Job,
        asset_id: i64,
        kinds: &[StepKind],
        fallback_lang: &str,
        res: ExternalResult,
        configured_mode: Option<String>,
    ) -> Result<Flow, AiJobError> {
        let res = sanitize(res);
        let auth_mode = res.usage.auth_mode.clone().or(configured_mode);
        let provider = res.provider.clone();
        let (kinds_owned, attempts, fallback_lang) =
            (kinds.to_vec(), job.attempts + 1, fallback_lang.to_string());
        let rejected = self.write(move |c, clock| {
            let kinds = kinds_owned;
            let known = c
                .query_row(
                    "SELECT 1 FROM media_assets WHERE asset_id = ?1",
                    [asset_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            record_usage(
                c,
                &UsageEntry {
                    provider: res.provider.clone(),
                    model: res.model.clone(),
                    kind: join_kinds(&kinds),
                    asset_id: known.then_some(asset_id),
                    input_tokens: res.usage.input_tokens,
                    output_tokens: res.usage.output_tokens,
                    request_bytes: res.usage.request_bytes,
                    images: res.usage.images,
                    ok: true,
                    error_code: None,
                    at: clock.now_ms(),
                    auth_mode,
                },
            )?;

            let has = |k| kinds.contains(&k);
            let new = NewAnalysis {
                fields: AnalysisFields {
                    description: has(StepKind::Describe).then(|| res.description.clone()),
                    keywords: has(StepKind::Keywords).then(|| res.keywords.clone()),
                    ocr_text: has(StepKind::Ocr).then(|| res.ocr_text.clone()),
                    summary: has(StepKind::Summary).then(|| res.summary.clone()),
                },
                language: if res.language.is_empty() {
                    fallback_lang
                } else {
                    res.language.clone()
                },
                provider: res.provider.clone(),
                model: res.model.clone(),
            };
            let tx = c.transaction()?;
            match new.save_in_tx(&tx, clock, asset_id) {
                Ok(_) => {
                    finish_ok(&tx, clock, asset_id, &kinds, &res.provider, attempts)?;
                    // 새 AI 텍스트가 의미 벡터에 반영되도록 완료된 embedding 을 다시 대기시킨다
                    tx.execute(
                        "UPDATE ai_task_state SET state = 'queued', error_code = NULL, updated_at = ?2
                         WHERE asset_id = ?1 AND kind = 'embedding' AND state = 'done'",
                        rusqlite::params![asset_id, clock.now_ms()],
                    )?;
                    tx.commit()?;
                    Ok(None)
                }
                Err(AiStoreError::NotFound(_)) => {
                    skip_inactive(&tx, clock.now_ms(), asset_id, &kinds)?;
                    tx.commit()?;
                    Ok(None)
                }
                // tx 는 drop 으로 롤백된다 (사용량은 이미 커밋됨)
                Err(AiStoreError::Invalid(why)) => Ok(Some(why)),
                Err(e) => Err(e.into()),
            }
        })?;
        if let Some(why) = rejected {
            tracing::error!(asset_id, why, "sanitized AI result rejected by the store");
            let f = StepFailure::new("invalid_result", "AI 응답을 저장할 수 없습니다");
            return self.apply_failure(
                job,
                asset_id,
                kinds,
                &provider,
                &f,
                None,
                &mut RetryNote::default(),
            );
        }
        Ok(Flow::Next)
    }
}

fn join_kinds(kinds: &[StepKind]) -> String {
    kinds
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join("+")
}
