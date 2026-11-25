//! 플러그인 KV 질의 — 게이트웨이(`mam-plugin-gateway`)가 쓰는 계약 질의 형태 모음 (SQL 은 이 파일에만 둔다).
//!
//! 질의 형태가 계약이다(`tests/plugin_kv_eqp.rs` 가 인덱스 사용·임시 B-tree 없음을 확인): 부분 인덱스(`uq_plugin_kv_global`·`_asset`)는
//! 질의에 `asset_id IS NULL` / `asset_id = ?` 가 있어야 쓰이므로 전역·자산 질의를 따로 둔다. 목록은 `LIKE` 없이 키 범위로 훑는다.
//! 값 검증(JSON·크기)과 총량(Q8)은 호출자 몫 — 여기는 저장·조회만 한다. 네임스페이스(`plugin_ref` — 레지스트리에서 `plugin_id` 로 찾는다, 없으면 데이터 없음)는 항상 첫 조건이다.

use rusqlite::{Connection, OptionalExtension, params};

use crate::plugin_registry;

/// 행 비용 = 키 바이트 + 값 바이트 + 행 오버헤드(제안값, Q8)
pub const ROW_OVERHEAD: i64 = 64;

const COST: &str = "length(CAST(key AS BLOB)) + length(CAST(value AS BLOB)) + 64";

fn select(cols: &str, global: bool) -> String {
    let scope = if global { "asset_id IS NULL AND key = ?2" } else { "asset_id = ?2 AND key = ?3" };
    format!("SELECT {cols} FROM plugin_kv WHERE plugin_ref = ?1 AND {scope}")
}

/// `asset_id` 가 `None` 이면 플러그인 전역 키. 값은 JSON 텍스트
pub fn get(conn: &Connection, plugin_id: &str, asset_id: Option<i64>, key: &str) -> rusqlite::Result<Option<String>> {
    one(conn, "value", plugin_id, asset_id, key)
}

/// 기존 행의 비용(없으면 `None`) — 덮어쓰기 delta 계산용
pub fn row_cost(conn: &Connection, plugin_id: &str, asset_id: Option<i64>, key: &str) -> rusqlite::Result<Option<i64>> {
    one(conn, COST, plugin_id, asset_id, key)
}

fn one<T: rusqlite::types::FromSql>(conn: &Connection, cols: &str, plugin_id: &str, asset_id: Option<i64>, key: &str) -> rusqlite::Result<Option<T>> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin_id)? else { return Ok(None) };
    let mut stmt = conn.prepare_cached(&select(cols, asset_id.is_none()))?;
    match asset_id {
        None => stmt.query_row(params![plugin_ref, key], |r| r.get(0)),
        Some(a) => stmt.query_row(params![plugin_ref, a, key], |r| r.get(0)),
    }
    .optional()
}

/// 있으면 덮어쓰고 없으면 넣는다 (`updated_at` 은 호출자가 시계에서 읽어 준다)
pub fn upsert(conn: &Connection, plugin_id: &str, asset_id: Option<i64>, key: &str, value: &str, now_ms: i64) -> rusqlite::Result<()> {
    let plugin_ref = plugin_registry::ensure_rs(conn, plugin_id)?;
    match asset_id {
        None => conn.prepare_cached(
            "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, NULL, ?2, ?3, ?4)
             ON CONFLICT (plugin_ref, key) WHERE asset_id IS NULL DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?.execute(params![plugin_ref, key, value, now_ms])?,
        Some(a) => conn.prepare_cached(
            "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (plugin_ref, asset_id, key) WHERE asset_id IS NOT NULL DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )?.execute(params![plugin_ref, a, key, value, now_ms])?,
    };
    Ok(())
}

/// 지웠으면 `true` (없으면 `false` — 멱등)
pub fn delete(conn: &Connection, plugin_id: &str, asset_id: Option<i64>, key: &str) -> rusqlite::Result<bool> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin_id)? else { return Ok(false) };
    let n = match asset_id {
        None => conn.prepare_cached("DELETE FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NULL AND key = ?2")?.execute(params![plugin_ref, key])?,
        Some(a) => conn.prepare_cached("DELETE FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id = ?2 AND key = ?3")?.execute(params![plugin_ref, a, key])?,
    };
    Ok(n > 0)
}

/// `prefix` 로 시작하는 모든 문자열보다 큰 가장 작은 문자열 (UTF-8 바이트 순서 = 코드포인트 순서). 끝 문자가 최대값이면 그 문자를
/// 버리고 앞 문자를 올린다. 모두 최대값(또는 빈 문자열)이면 상한 없음.
pub fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let next = match last as u32 {
            0xD7FF => Some('\u{E000}'), // 서로게이트 구간은 char 가 아니다
            c => char::from_u32(c + 1),
        };
        if let Some(n) = next {
            chars.push(n);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

/// `list_keys` 가 쓰는 SQL — 조각은 모두 고정 문자열(입력은 바인딩만). 시험이 EXPLAIN QUERY PLAN 으로 확인한다
pub fn list_sql(global: bool, lo_inclusive: bool, has_hi: bool) -> String {
    let (scope, n) = if global { ("asset_id IS NULL", 2) } else { ("asset_id = ?2", 3) };
    let lo_op = if lo_inclusive { ">=" } else { ">" };
    let hi_sql = if has_hi { format!(" AND key < ?{}", n + 1) } else { String::new() };
    let limit_n = n + 1 + usize::from(has_hi);
    format!("SELECT key FROM plugin_kv WHERE plugin_ref = ?1 AND {scope} AND key {lo_op} ?{n}{hi_sql} ORDER BY key LIMIT ?{limit_n}")
}

/// 키 목록(키셋 페이지) — `key > cursor`(또는 `key >= prefix`) 부터 `prefix` 범위 안에서 `limit` 개. 키 순서(바이트).
pub fn list_keys(conn: &Connection, plugin_id: &str, asset_id: Option<i64>, prefix: Option<&str>, cursor: Option<&str>, limit: usize) -> rusqlite::Result<Vec<String>> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin_id)? else { return Ok(Vec::new()) };
    let prefix = prefix.filter(|p| !p.is_empty());
    // 하한: 커서가 접두사 범위 안이면 `key > cursor`, 아니면 `key >= prefix`, 둘 다 없으면 `key > ''` (키는 비어 있지 않다)
    let (lo, lo_inclusive) = match (cursor, prefix) {
        (Some(c), Some(p)) if c >= p => (c, false),
        (_, Some(p)) => (p, true),
        (Some(c), None) => (c, false),
        (None, None) => ("", false),
    };
    let hi = prefix.and_then(prefix_upper_bound);
    let sql = list_sql(asset_id.is_none(), lo_inclusive, hi.is_some());
    let mut stmt = conn.prepare_cached(&sql)?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut binds: Vec<rusqlite::types::Value> = vec![plugin_ref.into()];
    if let Some(a) = asset_id {
        binds.push(a.into());
    }
    binds.push(lo.to_owned().into());
    if let Some(h) = hi {
        binds.push(h.into());
    }
    binds.push(limit.into());
    stmt.query_map(rusqlite::params_from_iter(binds), |r| r.get(0))?.collect()
}

/// 플러그인의 총 사용량(행 비용 합) — 자산·전역 행을 따로 훑어 각 부분 인덱스를 쓴다. 세션 첫 `kv.set`·재계산 때만 호출
pub fn sum_cost(conn: &Connection, plugin_id: &str) -> rusqlite::Result<i64> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin_id)? else { return Ok(0) };
    let mut total = 0i64;
    for scope in ["asset_id IS NULL", "asset_id IS NOT NULL"] {
        let sql = format!("SELECT COALESCE(SUM({COST}), 0) FROM plugin_kv WHERE plugin_ref = ?1 AND {scope}");
        let n: i64 = conn.prepare_cached(&sql)?.query_row([plugin_ref], |r| r.get(0))?;
        total = total.saturating_add(n);
    }
    Ok(total)
}

/// 자산에 붙은 태그 수 (`asset_tags` 기본키 인덱스) — 자산당 상한 검사용
pub fn count_asset_tags(conn: &Connection, asset_id: i64) -> rusqlite::Result<i64> {
    conn.prepare_cached("SELECT COUNT(*) FROM asset_tags WHERE asset_id = ?1")?.query_row([asset_id], |r| r.get(0))
}

#[cfg(test)]
mod tests {
    use super::prefix_upper_bound;

    // 분류: Boundary

    #[test]
    fn t_pdb_kv_01_prefix_upper_bound_cases() {
        assert_eq!(prefix_upper_bound("ab").as_deref(), Some("ac"));
        assert_eq!(prefix_upper_bound("a%").as_deref(), Some("a&"));
        assert_eq!(prefix_upper_bound("가").as_deref(), Some("각"));
        assert_eq!(prefix_upper_bound("a\u{10FFFF}").as_deref(), Some("b"));
        assert_eq!(prefix_upper_bound("\u{10FFFF}"), None);
        assert_eq!(prefix_upper_bound(""), None);
        assert_eq!(prefix_upper_bound("\u{D7FF}").as_deref(), Some("\u{E000}"));
    }
}
