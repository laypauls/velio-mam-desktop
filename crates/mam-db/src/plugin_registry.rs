//! 플러그인 레지스트리 (`plugins`·`plugin_providers`) — 문자열 ID 를 이 DB 안의 정수 키(`plugin_ref`·`provider_ref`)로 바꾼다.
//!
//! 규칙: 정수 키는 DB 안에서만 의미가 있다 — 설정·파일·IPC·로그·메모리 캐시에 두지 않는다(복원이 DB 파일을 통째로 바꾼다).
//! 레지스트리 행은 지우거나 바꾸지 않는다 — 같은 `plugin_id` 는 한 DB 안에서 영원히 같은 키다. 쓰는 쪽은 [`ensure`]
//! (없으면 만든다), 읽는 쪽은 [`lookup`] (없으면 `None` — 행을 만들지 않는다). 질의는 모두 자동 인덱스 커버링 SEARCH 다
//! (`tests/plugin_registry.rs` 가 EXPLAIN QUERY PLAN 으로 확인).

use rusqlite::{Connection, OptionalExtension};

use crate::plugin_settings::{InvalidPluginId, PutError, valid_id};

pub const LOOKUP_SQL: &str = "SELECT plugin_ref FROM plugins WHERE plugin_id = ?1";
pub const LOOKUP_PROVIDER_SQL: &str =
    "SELECT provider_ref FROM plugin_providers WHERE plugin_ref = ?1 AND name = ?2";
pub const ALL_SQL: &str = "SELECT plugin_ref, plugin_id FROM plugins ORDER BY plugin_id";

/// 플러그인의 정수 키 (없으면 `None` — 아직 데이터를 쓴 적 없다)
pub fn lookup(conn: &Connection, plugin_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.prepare_cached(LOOKUP_SQL)?
        .query_row([plugin_id], |r| r.get(0))
        .optional()
}

/// 플러그인의 정수 키를 돌려주고, 없으면 만든다. 잘못된 ID(빈 값·`:` 포함)는 거부한다.
pub fn ensure(conn: &Connection, plugin_id: &str) -> Result<i64, PutError> {
    if let Some(r) = lookup(conn, plugin_id)? {
        return Ok(r);
    }
    if !valid_id(plugin_id) {
        return Err(InvalidPluginId.into());
    }
    conn.prepare_cached("INSERT INTO plugins (plugin_id) VALUES (?1)")?
        .execute([plugin_id])?;
    Ok(conn.last_insert_rowid())
}

/// [`ensure`] 의 `rusqlite::Result` 판 — 잘못된 ID 는 `InvalidParameterName` (게이트웨이가 이미 검증하므로 방어용)
pub(crate) fn ensure_rs(conn: &Connection, plugin_id: &str) -> rusqlite::Result<i64> {
    ensure(conn, plugin_id).map_err(|e| match e {
        PutError::Db(e) => e,
        PutError::Id(_) => {
            rusqlite::Error::InvalidParameterName(format!("invalid plugin id: {plugin_id:?}"))
        }
    })
}

/// provider 의 정수 키 (없으면 `None`)
pub fn lookup_provider(
    conn: &Connection,
    plugin_ref: i64,
    name: &str,
) -> rusqlite::Result<Option<i64>> {
    conn.prepare_cached(LOOKUP_PROVIDER_SQL)?
        .query_row(rusqlite::params![plugin_ref, name], |r| r.get(0))
        .optional()
}

/// provider 의 정수 키를 돌려주고, 없으면 만든다. 빈 이름은 CHECK 가 거부한다.
pub fn ensure_provider(conn: &Connection, plugin_ref: i64, name: &str) -> rusqlite::Result<i64> {
    if let Some(r) = lookup_provider(conn, plugin_ref, name)? {
        return Ok(r);
    }
    conn.prepare_cached("INSERT INTO plugin_providers (plugin_ref, name) VALUES (?1, ?2)")?
        .execute(rusqlite::params![plugin_ref, name])?;
    Ok(conn.last_insert_rowid())
}

/// 모든 `(plugin_ref, plugin_id)` — `plugin_id` 순 (작은 표)
pub fn all(conn: &Connection) -> rusqlite::Result<Vec<(i64, String)>> {
    conn.prepare_cached(ALL_SQL)?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}
