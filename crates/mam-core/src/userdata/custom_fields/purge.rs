//! 필드·선택지 청크 삭제의 실제 정리 (결정 13·17). 삭제 요청은 `deleting_at` 만 찍는 짧은 트랜잭션이고,
//! 값 삭제는 여기서 짧은 트랜잭션 여러 개로 나눠 한다 — 80만 값 필드를 한 트랜잭션으로 지우면 쓰기 잠금이
//! BUSY_TIMEOUT 을 넘어 다른 쓰기가 실패한다 (PoC 측정).
//!
//! 청크 하나 = 자산 최대 [`METADATA_PURGE_CHUNK`] 개의 값 삭제 + 그 자산의 revision 증가 + 저우선순위(1) outbox,
//! 같은 트랜잭션. 중단돼도 `deleting_at` 이 남아 있으므로 다음 실행이 이어서 끝낸다(멱등).

use std::time::Duration;

use mam_db::Priority;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::bump_low_priority;
use crate::scan::IngestError;
use crate::userdata::{UserData, returning_ids};

/// 청크 하나가 다루는 자산 수 (운영값). 시험은 작은 값을 `_with` 변형으로 넘긴다.
pub const METADATA_PURGE_CHUNK: usize = 5_000;
/// 청크 사이 양보 시간 — 사용자 편집(고우선 쓰기)이 끼어들 틈
const CHUNK_PAUSE: Duration = Duration::from_millis(20);

enum Target {
    Field(i64),
    Option { option_id: i64, field_id: i64 },
}

impl UserData {
    /// 삭제 중인 필드·선택지를 모두 끝낼 때까지 청크 단위로 처리한다 (앱 시작·삭제 요청 직후에 호출).
    /// 처리하며 revision 을 올린 자산 수(청크별 합)를 돌려준다. 할 일이 없으면 0 (멱등).
    /// 호출 스레드에서 블로킹한다 — UI 스레드가 아닌 곳에서 부를 것.
    pub fn purge_pending_metadata_deletions(&self) -> Result<usize, IngestError> {
        self.purge_pending_metadata_deletions_with(METADATA_PURGE_CHUNK, CHUNK_PAUSE)
    }

    pub fn purge_pending_metadata_deletions_with(
        &self,
        chunk: usize,
        pause: Duration,
    ) -> Result<usize, IngestError> {
        let mut total = 0;
        while let Some(n) = self.purge_metadata_chunk(chunk)? {
            total += n;
            std::thread::sleep(pause);
        }
        Ok(total)
    }

    /// 청크 하나를 처리한다. 대기 중인 삭제가 없으면 `None`, 있으면 이 청크에서 revision 을 올린 자산 수.
    /// 필드 삭제를 선택지 삭제보다 먼저 처리한다. 대상의 값이 `chunk` 개 미만이 남았으면 같은 트랜잭션에서
    /// 필드/선택지 행까지 지운다.
    pub fn purge_metadata_chunk(&self, chunk: usize) -> Result<Option<usize>, IngestError> {
        let limit = i64::try_from(chunk.max(1)).unwrap_or(i64::MAX);
        let now = self.clock.now_ms();
        self.pool.write(Priority::Low, move |c| {
            Ok((|| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let Some(target) = next_target(&tx)? else {
                    return Ok(None);
                };
                let (field_id, option_id) = match target {
                    Target::Field(f) => (f, None),
                    Target::Option { option_id, field_id } => (field_id, Some(option_id)),
                };
                let ids = returning_ids(
                    &tx,
                    "SELECT DISTINCT asset_id FROM asset_metadata_values
                     WHERE field_id = ?1 AND (?2 IS NULL OR value_int = ?2) ORDER BY asset_id LIMIT ?3",
                    params![field_id, option_id, limit],
                )?;
                if !ids.is_empty() {
                    tx.execute(
                        "DELETE FROM asset_metadata_values
                         WHERE field_id = ?1 AND (?2 IS NULL OR value_int = ?2)
                           AND asset_id IN (SELECT value FROM json_each(?3))",
                        params![field_id, option_id, crate::userdata::json_ids(&ids)],
                    )?;
                    bump_low_priority(&tx, now, &ids)?;
                }
                if i64::try_from(ids.len()).is_ok_and(|n| n < limit) {
                    match option_id {
                        Some(o) => tx.execute("DELETE FROM metadata_field_options WHERE option_id = ?1", [o])?,
                        // 선택지·남은 값은 CASCADE (값은 위에서 이미 비었다)
                        None => tx.execute("DELETE FROM metadata_fields WHERE field_id = ?1", [field_id])?,
                    };
                }
                tx.commit()?;
                Ok(Some(ids.len()))
            })())
        })?
    }
}

fn next_target(c: &rusqlite::Connection) -> Result<Option<Target>, IngestError> {
    let field: Option<i64> = c
        .query_row(
            "SELECT field_id FROM metadata_fields WHERE deleting_at IS NOT NULL ORDER BY field_id LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(field_id) = field {
        return Ok(Some(Target::Field(field_id)));
    }
    Ok(c.query_row(
        "SELECT option_id, field_id FROM metadata_field_options WHERE deleting_at IS NOT NULL
         ORDER BY option_id LIMIT 1",
        [],
        |r| Ok(Target::Option { option_id: r.get(0)?, field_id: r.get(1)? }),
    )
    .optional()?)
}
