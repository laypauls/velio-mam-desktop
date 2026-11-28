//! 파생물(썸네일·프레임 스트립) 대기 상한 - 첫 대기부터 [`WAIT_CAP_MS`] 가 지나도 파생물이 없으면 `thumbnail_missing`
//! (재시도 가능 실패)으로 전환한다. 파생물 작업이 영영 만들어지지 않는 경우(원본 오프라인 등)에 30초 간격 대기가 끝없이 이어지지 않게 한다.
//!
//! 첫 대기 시각은 `ai_task_state.wait_since` (V31)에 둔다. `updated_at` 은 등록 시각이라 앞선 작업을 기다린 시간이 섞여 쓸 수 없다.
//! 단계 상태가 바뀌면(`state::set_step`·`upsert_queued`) 지워지므로 대기 -> 재시도 실패 -> 다음 대기는 새로 센다.

use rusqlite::params;

use super::AiJobError;
use super::executor::AiExecutor;
use super::failure::RetryNote;
use super::types::{StepFailure, StepKind};
use crate::jobs::Job;

/// 파생물을 기다리는 최대 시간 (첫 대기부터).
pub(super) const WAIT_CAP_MS: i64 = 30 * 60 * 1000;

impl AiExecutor {
    /// 대기 중인 단계(외부 분석·임베딩, queued)의 첫 대기 시각을 기록하고, 상한을 넘긴 단계는 `thumbnail_missing` 재시도 실패로
    /// 처리한다. 하나라도 넘겼으면 `true` (이때 `note` 에 재시도 사유가 쌓이므로 호출자는 더 미루지 않는다).
    pub(super) fn expire_waiting_steps(
        &self,
        job: &Job,
        asset_id: i64,
        note: &mut RetryNote,
    ) -> Result<bool, AiJobError> {
        let waiting = self.write(move |c, clock| {
            let now = clock.now_ms();
            let tx = c.transaction()?;
            tx.execute(
                "UPDATE ai_task_state SET wait_since = ?2
                 WHERE asset_id = ?1 AND state = 'queued' AND wait_since IS NULL
                   AND kind IN ('describe', 'keywords', 'ocr', 'summary', 'embedding')",
                params![asset_id, now],
            )?;
            let mut stmt = tx.prepare(
                "SELECT kind, wait_since FROM ai_task_state
                 WHERE asset_id = ?1 AND state = 'queued' AND wait_since IS NOT NULL
                   AND kind IN ('describe', 'keywords', 'ocr', 'summary', 'embedding')",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([asset_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            drop(stmt);
            tx.commit()?;
            Ok(rows
                .into_iter()
                .filter(|(_, since)| now.saturating_sub(*since) >= WAIT_CAP_MS)
                .filter_map(|(k, _)| StepKind::parse(&k))
                .collect::<Vec<_>>())
        })?;
        if waiting.is_empty() {
            return Ok(false);
        }
        tracing::warn!(
            asset_id,
            "derivative wait cap exceeded; failing with thumbnail_missing"
        );
        let failure = StepFailure::new("thumbnail_missing", "썸네일이 아직 만들어지지 않았습니다");
        let (external, embedding): (Vec<StepKind>, Vec<StepKind>) =
            waiting.into_iter().partition(|k| k.is_external());
        if !external.is_empty() {
            let provider = self.d.external.provider();
            self.apply_failure(job, asset_id, &external, &provider, &failure, None, note)?;
        }
        if !embedding.is_empty() {
            self.apply_failure(job, asset_id, &embedding, "local", &failure, None, note)?;
        }
        Ok(true)
    }
}
