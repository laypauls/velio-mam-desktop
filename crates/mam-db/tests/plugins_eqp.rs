//! 플러그인 표 질의의 인덱스 사용 계약 (T-pdb-06) — 질의 형태(리터럴 상태값·`IS [NOT] NULL`)가 계약이다.
//!
//! 분류: Performance 계약 (EXPLAIN QUERY PLAN — 100만 건 실측은 P5). SEARCH 만 허용(SCAN 없음), 정렬 질의는 임시 B-tree 없음.

mod common;

use common::migrated_db;
use rusqlite::Connection;

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    // 위치 매개변수마다 값을 묶는다 (질의마다 `?N` 은 한 번씩만 쓴다)
    let binds = sql.matches('?').count();
    stmt.query_map(
        rusqlite::params_from_iter(std::iter::repeat_n(1i64, binds)),
        |r| r.get::<_, String>(3),
    )
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// `index` 를 쓰는 SEARCH 가 있고 SCAN 이 없다. `no_sort` 면 임시 B-tree 정렬도 없다.
fn assert_plan(conn: &Connection, sql: &str, index: &str, no_sort: bool) {
    let details = plan(conn, sql);
    let joined = details.join(" | ");
    assert!(
        details
            .iter()
            .any(|d| d.starts_with("SEARCH") && d.contains(index)),
        "expected SEARCH using {index}, got: {joined}"
    );
    assert!(
        !details.iter().any(|d| d.starts_with("SCAN")),
        "full scan: {joined}"
    );
    if no_sort {
        assert!(
            !joined.contains("TEMP B-TREE"),
            "temp b-tree sort: {joined}"
        );
    }
}

#[test]
fn t_pdb_06_plugin_queries_use_their_intended_indexes() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let cases: &[(&str, &str, bool)] = &[
        (
            "SELECT value FROM plugin_kv WHERE plugin_ref=?1 AND asset_id=?2 AND key=?3",
            "uq_plugin_kv_asset",
            false,
        ),
        (
            "SELECT value FROM plugin_kv WHERE plugin_ref=?1 AND asset_id IS NULL AND key=?2",
            "uq_plugin_kv_global",
            false,
        ),
        (
            "SELECT key,value FROM plugin_kv WHERE plugin_ref=?1 AND asset_id IS NULL AND key>?2 ORDER BY key LIMIT ?3",
            "uq_plugin_kv_global",
            true,
        ),
        (
            "SELECT key,value FROM plugin_kv WHERE plugin_ref=?1 AND asset_id=?2 AND key>?3 ORDER BY key LIMIT ?4",
            "uq_plugin_kv_asset",
            true,
        ),
        // 배치 삭제 질의는 프로덕션 상수를 그대로 검사한다
        (
            mam_db::plugin_data::DELETE_SQLS[0],
            "uq_plugin_kv_asset",
            false,
        ),
        (
            mam_db::plugin_data::DELETE_SQLS[1],
            "uq_plugin_kv_global",
            false,
        ),
        (
            "SELECT task_id, asset_id FROM plugin_tasks WHERE plugin_ref=?1 AND status='pending' ORDER BY task_id LIMIT ?2",
            "idx_plugin_tasks_pending",
            true,
        ),
        (
            "SELECT COUNT(*) FROM plugin_tasks WHERE plugin_ref=?1 AND status='pending'",
            "idx_plugin_tasks_pending",
            false,
        ),
        (
            mam_db::plugin_data::DELETE_SQLS[2],
            "uq_plugin_tasks_key",
            false,
        ),
        (
            "DELETE FROM plugin_kv WHERE asset_id IN (SELECT asset_id FROM media_assets)",
            "idx_plugin_kv_asset",
            false,
        ),
        (
            "DELETE FROM plugin_tasks WHERE asset_id IN (SELECT asset_id FROM media_assets)",
            "idx_plugin_tasks_asset",
            false,
        ),
        // 레지스트리 조회 (자동 인덱스 커버링)
        (
            "SELECT plugin_ref FROM plugins WHERE plugin_id=?1",
            "sqlite_autoindex_plugins_1",
            false,
        ),
        (
            "SELECT provider_ref FROM plugin_providers WHERE plugin_ref=?1 AND name=?2",
            "sqlite_autoindex_plugin_providers_1",
            false,
        ),
        // 복원 후 정리의 데이터 존재 확인 (플러그인당 첫 행에서 멈춤)
        (
            mam_db::plugin_data::HAS_KV_GLOBAL_SQL,
            "uq_plugin_kv_global",
            false,
        ),
        (
            mam_db::plugin_data::HAS_KV_ASSET_SQL,
            "uq_plugin_kv_asset",
            false,
        ),
        (
            mam_db::plugin_data::HAS_TASKS_SQL,
            "uq_plugin_tasks_key",
            false,
        ),
    ];

    // Act / Assert
    for (sql, index, no_sort) in cases {
        assert_plan(c, sql, index, *no_sort);
    }
}

#[test]
fn t_pref_12_registry_lookups_are_covering_and_joined_pending_uses_pk_probes() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;

    // Act
    let lookup = plan(c, mam_db::plugin_registry::LOOKUP_SQL).join(" | ");
    let lookup_p = plan(c, mam_db::plugin_registry::LOOKUP_PROVIDER_SQL).join(" | ");
    let all = plan(c, mam_db::plugin_registry::ALL_SQL).join(" | ");
    let joined = plan(c, mam_db::plugin_tasks::PENDING_FOR_ASSET_SQL);
    let joined_s = joined.join(" | ");

    // Assert
    assert!(
        lookup.contains("COVERING INDEX sqlite_autoindex_plugins_1"),
        "{lookup}"
    );
    assert!(
        lookup_p.contains("COVERING INDEX sqlite_autoindex_plugin_providers_1"),
        "{lookup_p}"
    );
    assert!(
        !all.contains("TEMP B-TREE"),
        "plugin_id order uses the unique index: {all}"
    );
    assert!(
        joined
            .iter()
            .any(|d| d.starts_with("SEARCH t") && d.contains("idx_plugin_tasks_asset")),
        "{joined_s}"
    );
    assert!(
        joined
            .iter()
            .any(|d| d.starts_with("SEARCH p") && d.contains("INTEGER PRIMARY KEY")),
        "{joined_s}"
    );
    assert!(
        joined
            .iter()
            .any(|d| d.starts_with("SEARCH v") && d.contains("INTEGER PRIMARY KEY")),
        "{joined_s}"
    );
    assert!(
        !joined_s.contains("SCAN") && !joined_s.contains("TEMP B-TREE"),
        "{joined_s}"
    );
}
