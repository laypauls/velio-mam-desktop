//! AI 등록 보조 - 실패 재시도·사용자 편집 저장·동의 철회 (`AiScheduler` 의 나머지 메서드, 동작은 `schedule.rs` 와 같은 구조).

use std::sync::Arc;

use mam_db::{DbError, Priority};
use rusqlite::{OptionalExtension, params};

use super::{AiJobError, AiScheduler};
use crate::ai_jobs::plan::{FAILED_SQL, QueueResult};
use crate::ai_store::{AnalysisEdit, AnalysisView, SEMANTIC_ONLY_CODE, update_analysis};

impl AiScheduler {
    /// 실패한 단계를 다시 대기시킨다 (수동 재시도). `ids = None` 이면 실패가 있는 모든 자산.
    /// 시도 한도로 `failed` 가 된 AI 작업 행은 지우고 새로 등록한다.
    pub fn retry_failed(&self, ids: Option<&[i64]>) -> Result<QueueResult, AiJobError> {
        let ids: Option<Vec<i64>> = ids.map(<[_]>::to_vec);
        let (res, requeued) = self.write(move |c, now| {
            let tx = c.transaction()?;
            let targets: Vec<i64> = match ids {
                Some(v) => v,
                None => {
                    let mut stmt = tx.prepare(FAILED_SQL)?;
                    stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
                }
            };
            let mut res = QueueResult::default();
            let mut requeued = Vec::new();
            for id in targets {
                let n = tx.execute(
                    "UPDATE ai_task_state SET state = 'queued', error_code = NULL, attempts = 0, updated_at = ?2, origin = 'manual'
                     WHERE asset_id = ?1 AND state = 'failed'",
                    params![id, now],
                )?;
                if n == 0 {
                    res.skipped += 1;
                    continue;
                }
                tx.execute(
                    "DELETE FROM jobs WHERE kind = 'ai' AND asset_id = ?1 AND status = 'failed'",
                    [id],
                )?;
                res.queued += 1;
                requeued.push(id);
            }
            tx.commit()?;
            Ok((res, requeued))
        })?;
        self.enqueue_jobs(&requeued)?;
        Ok(res)
    }

    /// 사용자 편집(설명·키워드) 저장. `ai_store::update_analysis`(고우선 outbox, 낡은 의미 벡터를 같은 트랜잭션에서
    /// 의미 전용으로 재대기) 뒤 재대기된 경우 자산의 AI 작업을 등록한다 (대기 작업이 있으면 중복 등록 안 함).
    /// 외부 분석은 요청하지 않는다 - 편집이 덮어써질 수 있기 때문이다.
    pub fn edit_analysis(
        &self,
        asset_id: i64,
        edit: &AnalysisEdit,
    ) -> Result<AnalysisView, AiJobError> {
        let (edit, clock) = (edit.clone(), Arc::clone(&self.clock));
        let (view, requeued) = self.pool.write(Priority::High, move |c| {
            Ok::<_, DbError>((|| {
                let view = update_analysis(c, &*clock, asset_id, &edit)?;
                let requeued = c
                    .query_row(
                        "SELECT 1 FROM ai_task_state WHERE asset_id = ?1 AND kind = 'embedding'
                           AND state = 'queued' AND error_code = ?2",
                        params![asset_id, SEMANTIC_ONLY_CODE],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                Ok::<_, AiJobError>((view, requeued))
            })())
        })??;
        if requeued {
            self.enqueue_jobs(&[asset_id])?;
        }
        Ok(view)
    }

    /// 동의 철회: 대기 중(queued)인 외부 분석 단계를 skipped(consent_required)로 바꾼다. 기존 결과는 보존하고,
    /// 이미 실행 중인 요청은 끝까지 간다. 바뀐 단계 수를 돌려준다.
    pub fn cancel_external_pending(&self) -> Result<usize, AiJobError> {
        self.write(|c, now| {
            Ok(c.execute(
                "UPDATE ai_task_state SET state = 'skipped', error_code = 'consent_required', updated_at = ?1
                 WHERE kind IN ('describe', 'keywords', 'ocr', 'summary') AND state = 'queued'",
                [now],
            )?)
        })
    }
}
