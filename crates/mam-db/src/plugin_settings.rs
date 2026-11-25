//! 플러그인 설정 키 (`settings` 테이블의 `plugin_*:<id>` 행) — 설치 정보·활성·허용 권한·변경 피드 커서.
//!
//! [`crate::settings`] 의 `DEFS` 밖에 둔다: `settings::load` 가 모르는 키로 무시하고, 프론트 `settings_update` 패치로
//! 권한을 줄 수 없다 (`password_hash`·`ai_key_*` 선례). 이 모듈의 함수로만 읽고 쓴다.
//!
//! 복원 규칙(Q7)은 복원 후처리(`src-tauri/src/restore/plugins.rs`)가 적용한다. 값의 의미 검증(권한 문자열이 아는 값인지)은
//! 권한 어휘를 가진 쪽(manifest crate)의 몫이다 — 여기서는 문자열 집합으로만 다룬다.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};

/// `{"version":"1.2.0","installedAt":<ms>}` — 현재 설치된 패키지 정보
pub const INSTALLED: &str = "plugin_installed";
/// `true` / `false` — 키 없음 = false
pub const ENABLED: &str = "plugin_enabled";
/// 정렬된 권한 문자열 JSON 배열
pub const GRANTED: &str = "plugin_granted";
/// 정수 `event_id` — 변경 피드가 마지막으로 읽은 `search_outbox` 위치
pub const FEED_CURSOR: &str = "plugin_feed_cursor";

/// 모든 접두어 (복원·철회가 한 번에 훑는다)
pub const ALL_PREFIXES: [&str; 4] = [INSTALLED, ENABLED, GRANTED, FEED_CURSOR];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid plugin id")]
pub struct InvalidPluginId;

/// 플러그인 ID 는 비어 있지 않고 `:` 를 포함하지 않는다 (`<prefix>:<id>` 키의 구분자 보호)
pub fn valid_id(plugin_id: &str) -> bool {
    !plugin_id.is_empty() && !plugin_id.contains(':')
}

/// `<prefix>:<plugin_id>` 키. 빈 ID·`:` 포함 ID 는 거부한다.
pub fn key(prefix: &str, plugin_id: &str) -> Result<String, InvalidPluginId> {
    if valid_id(plugin_id) {
        Ok(format!("{prefix}:{plugin_id}"))
    } else {
        Err(InvalidPluginId)
    }
}

/// 한 접두어의 모든 행 (플러그인 ID -> 원문). 키 범위 `[prefix:, prefix;)` 로 설정 PK 인덱스를 쓴다 (LIKE 이스케이프 불필요).
pub fn load_prefixed(conn: &Connection, prefix: &str) -> rusqlite::Result<BTreeMap<String, String>> {
    let (lo, hi) = (format!("{prefix}:"), format!("{prefix};"));
    let mut stmt =
        conn.prepare("SELECT key, value FROM settings WHERE key >= ?1 AND key < ?2 ORDER BY key")?;
    let rows = stmt.query_map(params![lo, hi], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (k, v) = row?;
        if let Some(id) = k.strip_prefix(&lo).filter(|id| valid_id(id)) {
            out.insert(id.to_owned(), v);
        }
    }
    Ok(out)
}


/// 한 플러그인의 한 행 원문 (없으면 `None`). 잘못된 ID 는 `None` — 읽기는 닫힘 쪽으로 실패한다.
pub fn get(conn: &Connection, prefix: &str, plugin_id: &str) -> rusqlite::Result<Option<String>> {
    let Ok(key) = key(prefix, plugin_id) else { return Ok(None) };
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
}

/// 한 행을 쓴다 (없으면 추가, 있으면 덮어쓴다). 잘못된 ID 는 거부한다.
pub fn put(conn: &Connection, prefix: &str, plugin_id: &str, raw: &str) -> Result<(), PutError> {
    let key = key(prefix, plugin_id)?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, raw],
    )?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum PutError {
    #[error(transparent)]
    Id(#[from] InvalidPluginId),
    #[error("sqlite error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// 한 접두어의 모든 행을 지운다. 지운 행 수를 돌려준다.
pub fn delete_prefixed(conn: &Connection, prefix: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM settings WHERE key >= ?1 AND key < ?2",
        params![format!("{prefix}:"), format!("{prefix};")],
    )
}

/// 모든 접두어의 한 플러그인 행을 지운다 (제거·복원 정리용).
pub fn delete_plugin(conn: &Connection, plugin_id: &str) -> Result<(), PutError> {
    for prefix in ALL_PREFIXES {
        conn.execute("DELETE FROM settings WHERE key = ?1", [key(prefix, plugin_id)?])?;
    }
    Ok(())
}

/// `plugin_enabled` 원문 -> 활성 여부. `true` 가 아니면(손상 포함) 비활성.
pub fn parse_enabled(raw: &str) -> bool {
    serde_json::from_str::<bool>(raw).unwrap_or(false)
}

/// `plugin_granted` 원문 -> 권한 문자열 집합. 문자열이 아닌 항목·손상 문서는 버린다 (거부 쪽).
pub fn parse_granted(raw: &str) -> BTreeSet<String> {
    serde_json::from_str::<Vec<serde_json::Value>>(raw)
        .map(|items| {
            items
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// 권한 집합 -> 저장 원문 (정렬된 JSON 배열 — `BTreeSet` 이 정렬을 보장한다)
pub fn granted_to_raw(granted: &BTreeSet<String>) -> String {
    serde_json::Value::from_iter(granted.iter().cloned()).to_string()
}
