//! 전사 저장 - 덮어쓰기 + revision 증가 + outbox upsert 를 한 트랜잭션으로.

use mam_db::Clock;
use rusqlite::{Connection, OptionalExtension, params};

use super::{AI_PRIORITY, AiStoreError, Result, SaveOutcome, TranscriptRecord};

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// `asset_id` 의 전사를 저장한다(기존 전사·구간은 덮어쓴다). 자산이 없거나 `active` 가 아니면
/// [`AiStoreError::NotFound`] 이고 아무것도 바뀌지 않는다.
pub fn save_transcript(
    conn: &mut Connection,
    clock: &dyn Clock,
    asset_id: i64,
    rec: &TranscriptRecord,
) -> Result<SaveOutcome> {
    let now = clock.now_ms();
    let tx = conn.transaction()?;

    let revision: i64 = tx
        .query_row(
            "UPDATE media_assets SET revision = revision + 1, updated_at = ?2
             WHERE asset_id = ?1 AND status = 'active'
             RETURNING revision",
            params![asset_id, now],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(AiStoreError::NotFound(asset_id))?;

    // 구간은 ON DELETE CASCADE 로 함께 삭제된다 (foreign_keys = ON)
    tx.execute("DELETE FROM transcripts WHERE asset_id = ?1", [asset_id])?;
    let text = rec
        .segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let transcript_id: i64 = tx.query_row(
        "INSERT INTO transcripts (asset_id, language, text, engine, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING transcript_id",
        params![asset_id, rec.language, text, rec.engine, rec.model, now],
        |r| r.get(0),
    )?;
    {
        let mut ins = tx.prepare(
            "INSERT INTO transcript_segments (transcript_id, seq, start_ms, end_ms, text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (seq, s) in rec.segments.iter().enumerate() {
            ins.execute(params![
                transcript_id,
                seq as i64,
                to_i64(s.start_ms),
                to_i64(s.end_ms),
                s.text
            ])?;
        }
    }

    tx.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, 'upsert', ?3, ?4)",
        params![asset_id, revision, AI_PRIORITY, now],
    )?;
    tx.commit()?;
    Ok(SaveOutcome { revision })
}
