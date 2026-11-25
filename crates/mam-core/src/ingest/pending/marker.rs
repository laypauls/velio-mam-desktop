//! 비정상 종료 표식 (결정 Q) — 시작할 때 `settings` 에 표식을 남기고 정상 종료에서 지운다.
//! 시작 시 표식이 이미 있으면 직전 실행이 정상 종료하지 못한 것이다 — 그 사이 실시간 감시가 놓친 변경은
//! 재스캔만이 만회할 수 있으므로 호출자가 실시간 감시 폴더를 재스캔한다.
//!
//! DB 의 `settings` 행이라 관리 폴더(원본) 안에는 아무것도 쓰지 않는다. 키는 `Settings` 정의 밖이라 [`mam_db::settings::load`]
//! 에 보이지 않고 프론트 패치로 읽거나 쓸 수 없다 (`password_hash` 와 같은 방식).

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::scan::IngestError;

const KEY: &str = "ingest_run_marker";

/// 표식을 남긴다. 이미 있었으면(= 직전 실행이 정상 종료하지 못함) `true`. 시계는 호출자가 넘긴다.
pub fn begin(conn: &mut Connection, now_ms: i64) -> Result<bool, IngestError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<String> = tx
        .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| {
            r.get(0)
        })
        .optional()?;
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY, format!(r#"{{"startedAt":{now_ms}}}"#)],
    )?;
    tx.commit()?;
    Ok(existing.is_some())
}

/// 정상 종료 — 표식을 지운다.
pub fn clear(conn: &Connection) -> Result<(), IngestError> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [KEY])?;
    Ok(())
}
