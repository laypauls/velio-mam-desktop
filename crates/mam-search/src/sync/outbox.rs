//! `search_outbox` / `index_progress` 접근 (SQL 만 — 정책은 `worker`).

use std::collections::HashMap;

use rusqlite::{Connection, params};

use super::load::{APPLIED_BY_IDS_SQL, json_ids};

#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub event_id: i64,
    pub asset_id: u64,
    pub revision: u64,
    pub priority: i64,
    pub created_at: i64,
}

/// `pending` 이벤트를 `priority, event_id` 순으로 (인덱스 `idx_search_outbox_pending`, 정렬 임시 B-tree 없음).
pub const SELECT_PENDING_SQL: &str =
    "SELECT event_id, asset_id, revision, priority, created_at FROM search_outbox
         WHERE status = 'pending' ORDER BY priority, event_id LIMIT ?1";

pub(crate) fn select_pending(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<Event>> {
    let mut stmt = conn.prepare_cached(SELECT_PENDING_SQL)?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(Event {
            event_id: r.get(0)?,
            asset_id: r.get::<_, i64>(1)? as u64,
            revision: r.get::<_, i64>(2)? as u64,
            priority: r.get(3)?,
            created_at: r.get(4)?,
        })
    })?;
    rows.collect()
}

pub(crate) fn pending_count(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT count(*) FROM search_outbox WHERE status = 'pending'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as u64)
}

pub(crate) fn applied_revisions(
    conn: &Connection,
    index_name: &str,
    ids: &[u64],
) -> rusqlite::Result<HashMap<u64, u64>> {
    let mut stmt = conn.prepare_cached(APPLIED_BY_IDS_SQL)?;
    let rows = stmt.query_map(params![index_name, json_ids(ids)], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
    })?;
    rows.collect()
}

/// 단조 증가만 허용 (낮은 값으로 되돌리지 않는다).
pub(crate) fn raise_progress(
    conn: &Connection,
    index_name: &str,
    asset_id: u64,
    revision: u64,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO index_progress (index_name, asset_id, applied_revision) VALUES (?1, ?2, ?3)
         ON CONFLICT (index_name, asset_id) DO UPDATE
         SET applied_revision = MAX(applied_revision, excluded.applied_revision)",
    )?
    .execute(params![index_name, asset_id as i64, revision as i64])?;
    Ok(())
}

/// 재구축: 인덱스 내용을 SQLite 와 일치시켰으므로 값을 **그대로** 덮어쓴다.
pub(crate) fn set_progress(
    conn: &Connection,
    index_name: &str,
    asset_id: u64,
    revision: u64,
) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT INTO index_progress (index_name, asset_id, applied_revision) VALUES (?1, ?2, ?3)
         ON CONFLICT (index_name, asset_id) DO UPDATE SET applied_revision = excluded.applied_revision",
    )?
    .execute(params![index_name, asset_id as i64, revision as i64])?;
    Ok(())
}

pub(crate) fn mark_done(conn: &Connection, event_id: i64) -> rusqlite::Result<()> {
    conn.prepare_cached("UPDATE search_outbox SET status = 'done' WHERE event_id = ?1")?
        .execute(params![event_id])?;
    Ok(())
}

/// 실패 1회 기록. 반환: 이 호출로 `failed` 가 됐는가.
pub(crate) fn record_failure(
    conn: &Connection,
    event_id: i64,
    error: &str,
    max_attempts: u32,
) -> rusqlite::Result<bool> {
    conn.prepare_cached(
        "UPDATE search_outbox SET attempts = attempts + 1, last_error = ?2,
             status = CASE WHEN attempts + 1 >= ?3 THEN 'failed' ELSE 'pending' END
         WHERE event_id = ?1",
    )?
    .execute(params![event_id, error, max_attempts as i64])?;
    let status: String = conn.query_row(
        "SELECT status FROM search_outbox WHERE event_id = ?1",
        params![event_id],
        |r| r.get(0),
    )?;
    Ok(status == "failed")
}
