//! `plugin_tasks` 질의의 인덱스 사용 계약(T-pjob-21 EQP 절반)과 V26 마이그레이션 업그레이드(T-pref-11).
//!
//! 분류: Performance 계약 (EXPLAIN QUERY PLAN — 100만 건 실측은 M1) / Validation (V26 적용 목록·부분 인덱스) / Happy (집계).
//! 질의 형태가 계약이다: 부분 인덱스는 리터럴 상태값이 있어야 쓰인다. 실제 SQLite 에 마이그레이션을 적용해 쓴다.

mod common;

use common::{FixedClock, T0, TestDb, count, insert_asset, migrated_db, norm_sql, seed_task, shipped_migrations};
use mam_db::plugin_tasks::*;
use mam_db::{latest_version, migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    // `?N` 의 가장 큰 N 만큼 값을 묶는다 (EXPLAIN 은 값을 평가하지 않는다)
    let binds = (1..=9).rev().find(|n| sql.contains(&format!("?{n}"))).unwrap_or(0);
    stmt.query_map(rusqlite::params_from_iter(std::iter::repeat_n(1i64, binds)), |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// `expect` 를 포함한 SEARCH 가 있고, 실제 표(plugin_tasks·media_assets·search_outbox·jobs)의 전체 SCAN 이 없다
fn assert_indexed(conn: &Connection, sql: &str, expect: &str) {
    let details = plan(conn, sql);
    let joined = details.join(" | ");
    assert!(details.iter().any(|d| d.starts_with("SEARCH") && d.contains(expect)), "expected SEARCH using {expect}, got: {joined}");
    let full_scan = |d: &String| d.starts_with("SCAN") && ["plugin_tasks", "media_assets", "search_outbox", "jobs"].iter().any(|t| d.contains(t));
    assert!(!details.iter().any(full_scan), "full scan: {joined}");
}

#[test]
fn t_pjob_21_every_plugin_tasks_query_uses_its_index_without_full_scans() {
    // Arrange
    let db = migrated_db();
    let pk = "INTEGER PRIMARY KEY";
    let cases: &[(&str, &str)] = &[
        (FEED_SQL, "o USING INTEGER PRIMARY KEY (rowid>"),
        (FEED_SQL, "a USING INTEGER PRIMARY KEY (rowid=?)"),
        (RUN_ALL_SQL, "idx_media_assets_type_status"),
        (RUN_ALL_PAGE_SQL, pk),
        (PENDING_FOR_ASSET_SQL, "idx_plugin_tasks_asset"),
        (PENDING_COUNT_SQL, "idx_plugin_tasks_pending"),
        (FAILED_COUNT_SQL, "idx_plugin_tasks_failed"),
        (RETRY_FAILED_SQL, "idx_plugin_tasks_failed"),
        (MARK_INACTIVE_SQL, "idx_plugin_tasks_asset"),
        (COMPLETE_SQL, pk),
        (RETRY_LATER_SQL, pk),
        (FAIL_SQL, pk),
        (DELETE_TASK_SQL, pk),
        ("SELECT task_id, asset_id FROM plugin_tasks WHERE plugin_ref=?1 AND status='pending' ORDER BY task_id LIMIT ?2", "idx_plugin_tasks_pending"),
        ("SELECT task_id FROM plugin_tasks WHERE plugin_ref=?1 AND status='failed' ORDER BY task_id LIMIT ?2", "idx_plugin_tasks_failed"),
        ("SELECT COALESCE(MAX(event_id), 0) FROM search_outbox", "search_outbox"),
        ("SELECT revision FROM media_assets WHERE asset_id = ?1", pk),
        (ACTIVE_MEDIA_TYPE_SQL, pk),
        (PENDING_PAGE_SQL, "idx_plugin_tasks_pending"),
        (KIND_BUSY_SQL, "idx_jobs_kind_status"),
    ];

    // Act / Assert
    for (sql, expect) in cases {
        if *sql == "SELECT COALESCE(MAX(event_id), 0) FROM search_outbox" {
            let d = plan(&db.conn, sql).join(" | ");
            assert!(d.contains("search_outbox") && !d.contains("SCAN search_outbox"), "max_event_id: {d}");
            continue;
        }
        assert_indexed(&db.conn, sql, expect);
    }
}

#[test]
fn t_pjob_21_failed_listing_is_ordered_by_the_index_without_a_temp_sort() {
    let db = migrated_db();
    let d = plan(&db.conn, "SELECT task_id FROM plugin_tasks WHERE plugin_ref=?1 AND status='failed' ORDER BY task_id LIMIT ?2").join(" | ");
    assert!(!d.contains("TEMP B-TREE"), "{d}");
}

#[test]
fn t_pjob_21_pending_page_is_keyset_ordered_by_the_partial_index_and_the_active_type_lookup_is_a_primary_key_probe() {
    let db = migrated_db();
    let page = plan(&db.conn, PENDING_PAGE_SQL).join(" | ");
    assert!(page.contains("idx_plugin_tasks_pending") && !page.contains("TEMP B-TREE"), "{page}");
    let media = plan(&db.conn, ACTIVE_MEDIA_TYPE_SQL).join(" | ");
    assert!(media.starts_with("SEARCH") && media.contains("INTEGER PRIMARY KEY"), "{media}");
}

#[test]
fn t_pjob_21_counts_per_plugin_report_pending_and_failed_only_for_the_asked_plugins() {
    // Arrange: p1 에 pending 2·failed 1·done 1, p2 에 pending 1
    let db = migrated_db();
    let assets: Vec<i64> = (0..4).map(|_| insert_asset(&db.conn)).collect();
    for (plugin, asset, status) in [("p1", 0, "pending"), ("p1", 1, "pending"), ("p1", 2, "failed"), ("p1", 3, "done"), ("p2", 0, "pending")] {
        seed_task(&db.conn, plugin, "x", assets[asset], 1, status);
    }

    // Act
    let both = counts_by_plugin(&db.conn, &["p1", "p2", "ghost"]).unwrap();
    let only_p2 = counts_by_plugin(&db.conn, &["p2"]).unwrap();

    // Assert
    let row = |p: &str, pending, failed| PluginTaskCounts { plugin_id: p.to_owned(), pending, failed };
    assert_eq!(both, vec![row("p1", 2, 1), row("p2", 1, 0), row("ghost", 0, 0)]);
    assert_eq!(only_p2, vec![row("p2", 1, 0)]);
    assert_eq!((pending_count(&db.conn, "p1").unwrap(), failed_count(&db.conn, "p1").unwrap()), (2, 1));
    // 읽기는 레지스트리 행을 만들지 않는다 (T-pref-06)
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugins WHERE plugin_id = 'ghost'"), 0);
}

#[test]
fn t_pref_11_empty_db_is_at_v26_with_the_failed_partial_index() {
    // Arrange / Act
    let mut db = migrated_db();

    // Assert
    assert_eq!(latest_version(), 26);
    let sql: String = db.conn.query_row("SELECT sql FROM sqlite_master WHERE name = 'idx_plugin_tasks_failed'", [], |r| r.get(0)).unwrap();
    let sql = norm_sql(&sql);
    assert!(sql.contains("plugin_tasks (plugin_ref, task_id) WHERE status = 'failed'"), "{sql}");
    assert!(migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap().applied.is_empty());
}

#[test]
fn t_pref_11_v25_db_upgrades_to_v26_with_a_backup_and_empty_plugin_tables() {
    // Arrange: V25 DB (플러그인 표 없음) 에 자산 하나
    let dir = TempDir::new().unwrap();
    let (db_path, backup_dir) = (dir.path().join("mam.sqlite"), dir.path().join("backups"));
    let mut conn = open(&db_path).unwrap();
    let upto25: Vec<_> = shipped_migrations().into_iter().filter(|m| m.version() <= 25).collect();
    migrate_with(&mut conn, &upto25, &backup_dir, &FixedClock(T0)).unwrap();
    insert_asset(&conn);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('plugins', 'plugin_tasks')"), 0);
    let db = TestDb { dir, db_path, backup_dir, conn };
    let mut conn = db.conn;

    // Act
    let report = migrate(&mut conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert
    assert_eq!(report.applied, vec![26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM plugin_tasks") + count(&conn, "SELECT COUNT(*) FROM plugins"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM sqlite_master WHERE name = 'idx_plugin_tasks_failed'"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM pragma_foreign_key_check"), 0);
}

#[test]
fn t_pjob_21_every_plugin_tasks_sql_const_is_in_the_plan_cases_and_inline_queries_are_known() {
    // Arrange: 소스를 읽어 정적 검증한다 (CRLF 정규화 — project.md §5)
    let src = include_str!("../src/plugin_tasks.rs").replace("\r\n", "\n");
    let this = include_str!("plugin_tasks_eqp.rs").replace("\r\n", "\n");

    // Act
    let consts: Vec<&str> = src.lines().filter_map(|l| l.strip_prefix("pub const ")).filter_map(|r| r.split(':').next()).filter(|n| n.ends_with("_SQL")).collect();
    let inline = src.matches("prepare_cached(\"").count();

    // Assert: 새 `_SQL` 상수는 계획 사례 목록에 넣어야 하고, 상수가 아닌 인라인 질의(enqueue INSERT·max_event_id·asset_revision)가 늘면 알린다
    assert!(consts.len() >= 14, "found only {consts:?}");
    for name in &consts {
        assert!(this.contains(&format!("({name},")), "{name} has no EXPLAIN QUERY PLAN case in t_pjob_21_every_plugin_tasks_query_uses_its_index_without_full_scans");
    }
    assert_eq!(inline, 3, "a new inline query in plugin_tasks.rs needs a plan case (or a const) — enqueue INSERT, max_event_id, asset_revision are the known ones");
}
