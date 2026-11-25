//! "기존 보관 데이터 1회 정리" 완료 표시 — `settings` 테이블의 전용 키 `legacy_link_cleanup_done`.
//!
//! [`crate::settings`] 의 `DEFS` 밖에 둔다: 프론트 설정 패치로 읽거나 쓸 수 없다. 값이 `"1"` 이면 끝난 것이다.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::DbError;

/// `settings.key`
const KEY: &str = "legacy_link_cleanup_done";

/// 정리가 이미 끝났는가.
pub fn legacy_cleanup_done(conn: &Connection) -> Result<bool, DbError> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(raw.as_deref() == Some("1"))
}

/// 끝났다고 표시한다 (멱등).
pub fn mark_legacy_cleanup_done(conn: &Connection) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, '1')
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY],
    )?;
    Ok(())
}
