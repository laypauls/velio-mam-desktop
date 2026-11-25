//! 텍스트(AI 설명·키워드·전사)가 바뀌어 의미 벡터가 낡았음을 `ai_task_state` 에 표시한다.

use rusqlite::{Connection, params};

use super::Result;

/// `ai_task_state` 의 `embedding` 행이 `queued` + 이 `error_code` 이면 "의미 벡터만 다시 만든다" 는 표식이다.
/// 시각 벡터와 외부 분석은 건드리지 않는다.
pub const SEMANTIC_ONLY_CODE: &str = "semantic_only";

/// 완료(`done`)된 embedding 단계를 의미 전용으로 다시 대기시킨다. 바뀐 행이 있으면 `true`.
/// 이미 대기·실행 중이거나(다음 실행이 최신 텍스트를 읽는다) 실패·건너뜀·행 없음(시각 벡터가 없을 수 있다)이면
/// 건드리지 않는다. 호출자의 트랜잭션 안에서 부른다 (자산 변경과 한 트랜잭션).
pub fn requeue_semantic_embedding(conn: &Connection, now: i64, asset_id: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE ai_task_state SET state = 'queued', error_code = ?3, attempts = 0, updated_at = ?2
         WHERE asset_id = ?1 AND kind = 'embedding' AND state = 'done'",
        params![asset_id, now, SEMANTIC_ONLY_CODE],
    )?;
    Ok(n > 0)
}
