//! 플러그인 데이터(`plugin_kv`·`plugin_tasks`) 정리 — 설치되지 않은 플러그인의 행을 배치로 지운다.
//!
//! 데이터 소유자는 레지스트리(`plugins`)에서 찾는다 — 모든 데이터 행은 FK 로 레지스트리에 닿는다. 레지스트리 행은 지우지 않는다.
//! 질의 형태가 계약이다(`crates/mam-db/tests/plugins_eqp.rs` 가 인덱스 사용을 확인): 부분 인덱스는 질의 조건에
//! `asset_id IS [NOT] NULL` 이 있어야 쓰이므로 KV 는 자산 행·전역 행을 따로 훑는다. 배치마다 짧은 트랜잭션이라
//! 쓰기 연결을 오래 잡지 않는다 (호출자가 쓰기 우선순위를 정한다).

use std::collections::BTreeSet;

use rusqlite::{Connection, params};

use crate::plugin_registry;

/// 한 번에 지우는 최대 행 수
pub const BATCH: usize = 5_000;

/// 세 표(레지스트리·KV·작업)가 모두 있는가 — V26 이전 스키마의 복원 DB(마이그레이션 전)에는 없다
pub fn tables_exist(conn: &Connection) -> rusqlite::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('plugins', 'plugin_kv', 'plugin_tasks')",
        [],
        |r| r.get(0),
    )?;
    Ok(n == 3)
}

/// 데이터 존재 확인 질의 (플러그인당 세 번 — 각각 첫 행에서 멈춘다)
pub const HAS_KV_GLOBAL_SQL: &str = "SELECT 1 FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NULL LIMIT 1";
pub const HAS_KV_ASSET_SQL: &str = "SELECT 1 FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NOT NULL LIMIT 1";
pub const HAS_TASKS_SQL: &str = "SELECT 1 FROM plugin_tasks WHERE plugin_ref = ?1 LIMIT 1";

/// 두 표에 행이 있는 모든 플러그인 ID (설치 여부와 무관) — 레지스트리를 훑으며 플러그인마다 존재 확인
pub fn data_plugin_ids(conn: &Connection) -> rusqlite::Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    for (plugin_ref, plugin_id) in plugin_registry::all(conn)? {
        for sql in [HAS_KV_GLOBAL_SQL, HAS_KV_ASSET_SQL, HAS_TASKS_SQL] {
            if conn.prepare_cached(sql)?.exists([plugin_ref])? {
                ids.insert(plugin_id);
                break;
            }
        }
    }
    Ok(ids)
}

/// 한 플러그인의 행을 최대 `limit` 개씩(표마다) 지운다. 지운 행 수 합계 — 0 이면 더 없다 (레지스트리에 없으면 0).
pub fn delete_plugin_batch(conn: &Connection, plugin_id: &str, limit: usize) -> rusqlite::Result<usize> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin_id)? else { return Ok(0) };
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut total = 0;
    for sql in DELETE_SQLS {
        total += conn.execute(sql, params![plugin_ref, limit])?;
    }
    Ok(total)
}

/// 배치 삭제 질의 (시험이 EXPLAIN QUERY PLAN 으로 확인)
pub const DELETE_SQLS: [&str; 3] = [
    "DELETE FROM plugin_kv WHERE rowid IN
       (SELECT rowid FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NOT NULL LIMIT ?2)",
    "DELETE FROM plugin_kv WHERE rowid IN
       (SELECT rowid FROM plugin_kv WHERE plugin_ref = ?1 AND asset_id IS NULL LIMIT ?2)",
    "DELETE FROM plugin_tasks WHERE task_id IN
       (SELECT task_id FROM plugin_tasks WHERE plugin_ref = ?1 LIMIT ?2)",
];

/// `installed` 에 없는 플러그인의 KV·작업 행을 모두 지운다 (`BATCH` 행씩 배치마다 커밋 — 멱등, 중단 후 다시 호출해도 된다).
/// 표가 없으면(구 스키마) 아무것도 하지 않는다. 지운 행 수를 돌려준다.
pub fn purge_uninstalled(conn: &Connection, installed: &BTreeSet<String>) -> rusqlite::Result<usize> {
    purge_uninstalled_with(conn, installed, BATCH)
}

/// [`purge_uninstalled`] 의 배치 크기 지정판 (시험이 작은 배치로 반복 경로를 확인한다)
pub fn purge_uninstalled_with(
    conn: &Connection,
    installed: &BTreeSet<String>,
    batch: usize,
) -> rusqlite::Result<usize> {
    if !tables_exist(conn)? {
        return Ok(0);
    }
    let mut removed = 0;
    for id in data_plugin_ids(conn)?.difference(installed) {
        loop {
            let tx = conn.unchecked_transaction()?;
            let n = delete_plugin_batch(&tx, id, batch)?;
            tx.commit()?;
            if n == 0 {
                break;
            }
            removed += n;
        }
    }
    Ok(removed)
}
