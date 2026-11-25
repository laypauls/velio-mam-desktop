//! V26 `plugins` · `plugin_providers` · `plugin_kv` · `plugin_tasks` · `jobs.kind` 'plugin' 마이그레이션 (T-pdb-01 ~ 05, T-pref-02·03).
//!
//! 분류: Happy (빈 DB·재실행) / Business rule (V25 -> V26 jobs 보존, 자산 삭제 CASCADE, upsert 규칙) /
//! Validation (CHECK·UNIQUE·FK 제약)

mod common;

use common::{
    FixedClock, T0, TestDb, count, insert_asset, insert_volume, jobs_columns, jobs_dump,
    jobs_indexes, migrated_db, norm_sql, shipped_migrations,
};
use mam_db::plugin_registry::{ensure, ensure_provider};
use mam_db::{latest_version, migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v25_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto25: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 25)
        .collect();
    migrate_with(&mut conn, &upto25, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

/// `plugin_ref` 를 직접 받는 원시 삽입 (제약 시험용)
fn kv_ref(conn: &Connection, plugin_ref: i64, asset: Option<i64>, key: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, ?2, ?3, '1', 0)",
        (plugin_ref, asset, key),
    )
}

fn task_ref(conn: &Connection, plugin_ref: i64, provider_ref: i64, asset: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at)
         VALUES (?1, ?2, ?3, 1, 0)",
        (plugin_ref, provider_ref, asset),
    )
}

/// 레지스트리를 거쳐 넣는다 (플러그인·provider 가 없으면 만든다)
fn kv(conn: &Connection, plugin: &str, asset: Option<i64>, key: &str) -> rusqlite::Result<usize> {
    let pr = ensure(conn, plugin).map_err(|_| rusqlite::Error::InvalidQuery)?;
    kv_ref(conn, pr, asset, key)
}

fn task(conn: &Connection, plugin: &str, provider: &str, asset: i64) -> rusqlite::Result<usize> {
    let pr = ensure(conn, plugin).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let vr = ensure_provider(conn, pr, provider)?;
    task_ref(conn, pr, vr, asset)
}

#[test]
fn t_pdb_01_empty_db_has_strict_tables_partial_indexes_and_is_idempotent() {
    // Arrange / Act
    let mut db = migrated_db();

    // Assert
    assert_eq!(latest_version(), 27);
    for table in ["plugins", "plugin_providers", "plugin_kv", "plugin_tasks"] {
        let strict: bool = db
            .conn
            .query_row("SELECT strict FROM pragma_table_list(?1)", [table], |r| r.get(0))
            .unwrap();
        assert!(strict, "{table} must be STRICT");
    }
    let sql_of = |name: &str| -> String {
        let sql: String = db
            .conn
            .query_row("SELECT sql FROM sqlite_master WHERE name = ?1", [name], |r| r.get(0))
            .unwrap();
        norm_sql(&sql)
    };
    assert!(sql_of("uq_plugin_kv_global").ends_with("WHERE asset_id IS NULL"));
    assert!(sql_of("uq_plugin_kv_asset").ends_with("WHERE asset_id IS NOT NULL"));
    assert!(sql_of("idx_plugin_tasks_pending").ends_with("WHERE status = 'pending'"));
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    assert!(again.applied.is_empty());
}

#[test]
fn t_pdb_02_upgrade_from_v25_preserves_every_job_row_column_and_index() {
    // Arrange: V25 DB 에 모든 kind(V16 까지) x status 조합 + 폴더 연결 작업
    let mut db = v25_db();
    let vol = insert_volume(&db.conn, "v");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();
    let mut n = 0;
    for (ki, kind) in ["scan", "meta", "thumb", "hash", "index", "ai", "scene"]
        .iter()
        .enumerate()
    {
        for (si, status) in ["pending", "running", "failed"].iter().enumerate() {
            n += 1;
            let folder = (*kind == "scan").then_some(1);
            db.conn
                .execute(
                    "INSERT INTO jobs (job_id, kind, asset_id, folder_id, priority, status, attempts, not_before, last_error, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        100 + n,
                        kind,
                        if folder.is_some() { None } else { Some(1000 + n) },
                        folder,
                        10 * (ki as i64) + 5,
                        status,
                        si as i64,
                        7 * n,
                        (*status == "failed").then_some("boom"),
                        T0 + n,
                        T0 + 2 * n
                    ],
                )
                .unwrap();
        }
    }
    let (rows, cols, idx) = (
        jobs_dump(&db.conn),
        jobs_columns(&db.conn),
        jobs_indexes(&db.conn),
    );
    assert!(rows.len() >= 15, "{rows:?}");
    assert_eq!(idx.len(), 5);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert: 값·열·인덱스 동일, 백업 존재, FK ON·위반 0, 폴더 CASCADE 유지
    assert_eq!(report.applied, vec![26, 27]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(jobs_dump(&db.conn), rows);
    assert_eq!(jobs_columns(&db.conn), cols);
    assert_eq!(jobs_indexes(&db.conn), idx);
    assert_eq!(count(&db.conn, "PRAGMA foreign_keys"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM pragma_foreign_key_check"), 0);
    assert!(count(&db.conn, "SELECT COUNT(*) FROM jobs WHERE folder_id = 1") > 0);
    db.conn
        .execute("DELETE FROM managed_folders WHERE folder_id = 1", [])
        .unwrap();
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM jobs WHERE folder_id = 1"), 0);
}

#[test]
fn t_pdb_03_constraints_reject_bad_rows_and_allow_valid_combinations() {
    // Arrange
    let db = migrated_db();
    let (a, b) = (insert_asset(&db.conn), insert_asset(&db.conn));
    let c = &db.conn;
    let job = |kind: &str, asset: i64| {
        c.execute(
            "INSERT INTO jobs (kind, asset_id, created_at, updated_at) VALUES (?1, ?2, 0, 0)",
            (kind, asset),
        )
    };

    // Act / Assert: jobs.kind
    assert!(job("plugin", a).is_ok());
    assert!(job("plugin", a).is_err(), "pending (kind, asset) is still unique");
    assert!(job("bogus", a).is_err());
    // plugin_kv: 중복 거부 / 다른 자산·전역 vs 자산 허용
    assert!(kv(c, "p", None, "k").is_ok());
    assert!(kv(c, "p", None, "k").is_err(), "global duplicate");
    assert!(kv(c, "p", Some(a), "k").is_ok(), "global vs asset key may coincide");
    assert!(kv(c, "p", Some(a), "k").is_err(), "asset duplicate");
    assert!(kv(c, "p", Some(b), "k").is_ok(), "same key on another asset");
    assert!(kv(c, "q", None, "k").is_ok(), "same key for another plugin");
    // 빈 값·없는 자산 FK
    assert!(kv(c, "", None, "k").is_err());
    assert!(kv(c, "p", None, "").is_err());
    assert!(kv(c, "p", Some(999), "k2").is_err(), "FK");
    // plugin_tasks
    assert!(task(c, "p", "prov", a).is_ok());
    assert!(task(c, "p", "prov", a).is_err(), "(plugin, provider, asset) unique");
    assert!(task(c, "p", "other", a).is_ok());
    assert!(task(c, "", "prov", b).is_err());
    assert!(task(c, "p", "", b).is_err());
    assert!(task(c, "p", "prov", 999).is_err(), "FK");
    let (pr, vr) = (ensure(c, "p").unwrap(), ensure_provider(c, ensure(c, "p").unwrap(), "x").unwrap());
    let bad = |values: &str| {
        c.execute(
            &format!(
                "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, done_revision, status, updated_at) VALUES {values}"
            ),
            [],
        )
    };
    assert!(bad(&format!("({pr}, {vr}, {a}, 1, 0, 'bogus', 0)")).is_err());
    assert!(bad(&format!("({pr}, {vr}, {a}, 0, 0, 'pending', 0)")).is_err(), "target_revision >= 1");
    assert!(bad(&format!("({pr}, {vr}, {a}, 1, -1, 'pending', 0)")).is_err(), "done_revision >= 0");
    assert!(bad(&format!("({pr}, {vr}, {a}, 1, 0, 'failed', 0)")).is_ok());
}

#[test]
fn t_pdb_04_hard_deleting_an_asset_cascades_both_tables_but_keeps_global_kv() {
    // Arrange
    let db = migrated_db();
    let (a, b) = (insert_asset(&db.conn), insert_asset(&db.conn));
    for id in [a, b] {
        kv(&db.conn, "p", Some(id), "k").unwrap();
        task(&db.conn, "p", "prov", id).unwrap();
    }
    kv(&db.conn, "p", None, "g").unwrap();

    // Act
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = ?1", [a])
        .unwrap();

    // Assert
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugin_kv WHERE asset_id IS NOT NULL"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugin_kv WHERE asset_id IS NULL"), 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks"), 1);
    assert_eq!(
        count(&db.conn, &format!("SELECT COUNT(*) FROM plugin_tasks WHERE asset_id = {b}")),
        1
    );
}

const UPSERT_TASK: &str = "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at)
    VALUES (1, 1, ?1, ?2, 1)
    ON CONFLICT (plugin_ref, provider_ref, asset_id) DO UPDATE SET
        target_revision = MAX(target_revision, excluded.target_revision),
        status = CASE WHEN MAX(target_revision, excluded.target_revision) > done_revision THEN 'pending' ELSE status END,
        updated_at = excluded.updated_at";

#[test]
fn t_pdb_05_upserts_match_the_unique_indexes_and_never_lower_target_revision() {
    // Arrange
    let db = migrated_db();
    let c = &db.conn;
    let a = insert_asset(c);
    let pr = ensure(c, "p").unwrap();
    assert_eq!((pr, ensure_provider(c, pr, "prov").unwrap()), (1, 1), "UPSERT_TASK 상수가 쓰는 첫 정수 키");
    let row = |c: &Connection| -> (i64, String) {
        c.query_row("SELECT target_revision, status FROM plugin_tasks", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    };

    // Act / Assert: 작업 — 같은 키 한 행, target 감소 없음, target > done 일 때만 pending 복귀
    c.execute(UPSERT_TASK, (a, 3)).unwrap();
    c.execute("UPDATE plugin_tasks SET done_revision = 3, status = 'done'", [])
        .unwrap();
    c.execute(UPSERT_TASK, (a, 2)).unwrap();
    assert_eq!(row(c), (3, "done".into()), "older revision must not reopen or lower");
    c.execute(UPSERT_TASK, (a, 3)).unwrap();
    assert_eq!(row(c), (3, "done".into()), "same revision stays done");
    c.execute(UPSERT_TASK, (a, 5)).unwrap();
    assert_eq!(row(c), (5, "pending".into()));
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_tasks"), 1);

    // 전역 KV / 자산 KV — 부분 인덱스 ON CONFLICT 대상
    let global = "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (1, NULL, 'k', ?1, 1)
        ON CONFLICT (plugin_ref, key) WHERE asset_id IS NULL DO UPDATE SET value = excluded.value";
    let per_asset = "INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (1, ?2, 'k', ?1, 1)
        ON CONFLICT (plugin_ref, asset_id, key) WHERE asset_id IS NOT NULL DO UPDATE SET value = excluded.value";
    c.execute(global, ["1"]).unwrap();
    c.execute(global, ["2"]).unwrap();
    c.execute(per_asset, ("\"a\"", a)).unwrap();
    c.execute(per_asset, ("\"b\"", a)).unwrap();
    assert_eq!(count(c, "SELECT COUNT(*) FROM plugin_kv"), 2);
    let v: String = c
        .query_row("SELECT value FROM plugin_kv WHERE asset_id IS NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "2");
    let v: String = c
        .query_row("SELECT value FROM plugin_kv WHERE asset_id IS NOT NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, "\"b\"");
}
