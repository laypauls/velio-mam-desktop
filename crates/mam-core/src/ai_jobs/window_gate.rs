//! 예약 시간대 실행 게이트 - 백그라운드 스윕은 시간대 안에서만 등록하지만 이미 등록된 작업은 시간대가 끝난 뒤에도 남는다.
//! 실행 직전에 `background` 출처 단계만 남은 자산이 시간대 밖이면 **삭제하지 않고** 다음 시간대 시작까지 미룬다
//! (시도 횟수 불변). 수동·신규 자동·임베딩 백필 출처는 시간대와 무관하게 실행한다 (기존 정책).

use super::AiJobError;
use super::config::local_hour;
use super::executor::AiExecutor;
use super::origin::Origin;

impl AiExecutor {
    /// 시간대 밖이고 남은 단계가 모두 `background` 출처면 다음 시간대 시작 시각(epoch ms), 아니면 `None`.
    pub(super) fn outside_window_until(&self, asset_id: i64) -> Result<Option<i64>, AiJobError> {
        let schedule = self.d.settings.snapshot().schedule;
        let (now, offset) = (self.now(), (self.d.utc_offset)());
        if schedule.allows(local_hour(now, offset)) {
            return Ok(None);
        }
        let origins: Vec<Option<String>> = self.read(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT origin FROM ai_task_state
                 WHERE asset_id = ?1 AND state IN ('queued', 'running')",
            )?;
            Ok(stmt
                .query_map([asset_id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        })?;
        let only_background = !origins.is_empty()
            && origins
                .iter()
                .all(|o| o.as_deref() == Some(Origin::Background.as_str()));
        Ok(only_background.then(|| schedule.next_start_ms(now, offset)))
    }
}
