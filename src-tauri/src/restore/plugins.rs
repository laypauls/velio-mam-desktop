//! 복원·복구 후처리의 플러그인 부분 (Q7) — 설정 키 병합 규칙과 설치되지 않은 플러그인 데이터 정리.
//!
//! | 키 | 복원 규칙 |
//! |---|---|
//! | `plugin_installed:<id>` | 현재 값으로 대체 (`MAMData/plugins` 패키지는 복원으로 되감기지 않는다) |
//! | `plugin_enabled:<id>` | 현재 && 백업 (키 없음 = false) |
//! | `plugin_granted:<id>` | 현재 ∩ 백업 |
//! | `plugin_feed_cursor:<id>` | 백업 값 유지 (`search_outbox`·`plugin_tasks` 와 함께 되감긴다) |
//!
//! 현재 설치되지 않은 플러그인의 설정 키·`plugin_kv`·`plugin_tasks` 행은 지운다 (사용자 결정 2026-11-22). 데이터 소유자는 복원된 DB 자신의
//! 레지스트리(`plugins`)에서 찾는다 — 정수 키(`plugin_ref`)는 DB 마다 달라 현재 DB 에서 가져오지 않고, 설정(`plugin_id` 키)만 이어 받는다.

use std::collections::BTreeSet;

use mam_db::plugin_settings::{
    ENABLED, FEED_CURSOR, GRANTED, INSTALLED, PutError, delete_plugin, delete_prefixed,
    granted_to_raw, load_prefixed, parse_enabled, parse_granted, put,
};
use rusqlite::Connection;

use super::KeptPlugins;

/// 복원 요청 시점의 현재 플러그인 설정. 설치되지 않은 플러그인의 `enabled`·`granted` 는 의미가 없어 버린다.
pub fn read_plugins(conn: &Connection) -> rusqlite::Result<KeptPlugins> {
    let installed = load_prefixed(conn, INSTALLED)?;
    let enabled = load_prefixed(conn, ENABLED)?
        .into_iter()
        .filter(|(id, raw)| installed.contains_key(id) && parse_enabled(raw))
        .map(|(id, _)| id)
        .collect();
    let granted = load_prefixed(conn, GRANTED)?
        .into_iter()
        .filter(|(id, _)| installed.contains_key(id))
        .map(|(id, raw)| (id, parse_granted(&raw)))
        .collect();
    Ok(KeptPlugins {
        installed,
        enabled,
        granted,
    })
}

fn put_row(conn: &Connection, prefix: &str, id: &str, raw: &str) -> rusqlite::Result<()> {
    match put(conn, prefix, id, raw) {
        Ok(()) => Ok(()),
        Err(PutError::Db(e)) => Err(e),
        // `load_prefixed` 가 이미 걸러낸 ID 만 오므로 도달하지 않는다 — 그래도 행을 쓰지 않고 넘어간다 (거부 쪽)
        Err(PutError::Id(_)) => Ok(()),
    }
}

/// 복원된 DB 의 플러그인 설정 키에 위 규칙을 적용한다 (호출자의 트랜잭션 안에서, 멱등).
pub fn merge_plugins(conn: &Connection, kept: &KeptPlugins) -> rusqlite::Result<()> {
    let backup_enabled = load_prefixed(conn, ENABLED)?;
    let backup_granted = load_prefixed(conn, GRANTED)?;
    let backup_cursors = load_prefixed(conn, FEED_CURSOR)?;

    delete_prefixed(conn, INSTALLED)?;
    delete_prefixed(conn, ENABLED)?;
    delete_prefixed(conn, GRANTED)?;
    for (id, raw) in &kept.installed {
        put_row(conn, INSTALLED, id, raw)?;
        let enabled = kept.enabled.contains(id)
            && backup_enabled.get(id).is_some_and(|raw| parse_enabled(raw));
        put_row(conn, ENABLED, id, &enabled.to_string())?;
        let allowed: BTreeSet<String> = match (kept.granted.get(id), backup_granted.get(id)) {
            (Some(now), Some(then)) => now.intersection(&parse_granted(then)).cloned().collect(),
            _ => BTreeSet::new(),
        };
        put_row(conn, GRANTED, id, &granted_to_raw(&allowed))?;
    }
    for id in backup_cursors
        .keys()
        .filter(|id| !kept.installed.contains_key(*id))
    {
        let _ = delete_plugin(conn, id); // 잘못된 ID 는 `load_prefixed` 가 이미 거른다
    }
    Ok(())
}

/// 설치되지 않은 플러그인의 KV·작업 행을 5,000행씩 배치로 지운다 (배치마다 커밋). 복원된 DB 가 V26 이전 스키마면 할 일이 없다.
pub fn purge_uninstalled_data(conn: &Connection, kept: &KeptPlugins) -> rusqlite::Result<usize> {
    let installed: BTreeSet<String> = kept.installed.keys().cloned().collect();
    mam_db::plugin_data::purge_uninstalled(conn, &installed)
}

/// 자동 복구(원천 DB 손상) 후: 현재 설정을 알 수 없으므로 모든 플러그인을 비활성으로 돌리고 허용 권한을 지운다 —
/// 사용자가 다시 켜고 다시 허용해야 한다. 이전에 활성이던 플러그인 수를 돌려준다 (알림 플래그는 IPC 변경이라 P6/P7 로 미룬다).
pub fn withdraw_plugins(conn: &Connection) -> rusqlite::Result<usize> {
    let enabled = load_prefixed(conn, ENABLED)?;
    let was_enabled = enabled.values().filter(|raw| parse_enabled(raw)).count();
    for id in enabled.keys() {
        put_row(conn, ENABLED, id, "false")?;
    }
    delete_prefixed(conn, GRANTED)?;
    Ok(was_enabled)
}
