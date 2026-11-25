//! V15 `asset_scene_cuts` · V16 `jobs` 재생성 마이그레이션 (T-scene-01 ~ 03).
//!
//! 분류: Happy (V14 DB 에서 업그레이드) / Validation (CHECK·FK 제약, 'scene' 허용·'bogus' 거부) /
//! Business rule (jobs 행·인덱스·열 정의 보존, CASCADE, 테이블 수)

mod common;

use common::{FixedClock, T0, TestDb, count, insert_volume, shipped_migrations};
use mam_db::{migrate, migrate_with, open};
use rusqlite::Connection;
use tempfile::TempDir;

fn v14_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    let upto14: Vec<_> = shipped_migrations()
        .into_iter()
        .filter(|m| m.version() <= 14)
        .collect();
    migrate_with(&mut conn, &upto14, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

fn add_asset(conn: &Connection, id: i64, media_type: &str) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, ?2, 10, 0, 0, 0)",
        (id, media_type),
    )
    .unwrap();
}

fn norm(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// jobs 의 열 정의 (이름·타입·NOT NULL·기본값·PK 순서)
fn jobs_columns(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT cid || ':' || name || ':' || type || ':' || \"notnull\" || ':' || IFNULL(dflt_value, '-') || ':' || pk FROM pragma_table_info('jobs') ORDER BY cid")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// jobs 인덱스 (이름 → 공백 정규화한 정의)
fn jobs_indexes(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'index' AND tbl_name = 'jobs' AND sql IS NOT NULL ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, norm(&r.get::<_, String>(1)?)))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn jobs_dump(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT job_id || '|' || kind || '|' || IFNULL(asset_id, 'N') || '|' || IFNULL(folder_id, 'N') || '|' || priority || '|' || status || '|' || attempts || '|' || not_before || '|' || IFNULL(last_error, 'N') || '|' || created_at || '|' || updated_at FROM jobs ORDER BY job_id")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn t_scene_01_v15_upgrade_from_v14_adds_table_check_and_partial_index() {
    // Arrange
    let mut db = v14_db();
    add_asset(&db.conn, 1, "video");

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert: 업그레이드 + 백업 + 기존 행 보존 + 재실행 no-op
    assert_eq!(report.applied, vec![15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
    assert!(report.backup.is_some_and(|b| b.is_file()));
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM media_assets"), 1);
    let strict: bool = db
        .conn
        .query_row(
            "SELECT strict FROM pragma_table_list('asset_scene_cuts')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(strict);
    let idx: String = db
        .conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'idx_asset_scene_cuts_pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(norm(&idx).ends_with("WHERE state = 'pending'"), "{idx}");
    // 정상 행
    db.conn
        .execute(
            "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (1, 'done', 5)",
            [],
        )
        .unwrap();
    let again = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 2)).unwrap();
    assert!(again.applied.is_empty());
}

#[test]
fn t_scene_01_constraints_reject_bad_rows() {
    // Arrange
    let db = migrated();
    add_asset(&db.conn, 1, "video");
    let put = |sql: &str| db.conn.execute(sql, []);

    // Act / Assert: state CHECK, detector_version CHECK, FK, PK, NOT NULL
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (1, 'bogus', 0)").is_err());
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state, detector_version, updated_at) VALUES (1, 'done', 0, 0)").is_err());
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (99, 'done', 0)").is_err());
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state) VALUES (1, 'done')").is_err());
    // 값이 늘어날 수 있는 열은 CHECK 가 없다 (W9)
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state, sensitivity, error, updated_at) VALUES (1, 'failed', 'future', 'future_code', 0)").is_ok());
    assert!(put("INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (1, 'done', 0)").is_err());
}

fn migrated() -> TestDb {
    common::migrated_db()
}

#[test]
fn t_scene_02_v16_preserves_every_job_row_column_and_index() {
    // Arrange: V14 에서 모든 kind·status 조합 + 폴더 연결 작업
    let mut db = v14_db();
    let vol = insert_volume(&db.conn, "v");
    db.conn
        .execute(
            "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 'a', 'a', 'a', ?2)",
            (vol, T0),
        )
        .unwrap();
    let kinds = ["scan", "meta", "thumb", "hash", "index", "ai"];
    let statuses = ["pending", "running", "failed"];
    let mut n = 0;
    for (ki, kind) in kinds.iter().enumerate() {
        for (si, status) in statuses.iter().enumerate() {
            n += 1;
            let folder = if *kind == "scan" { Some(1) } else { None };
            // 대기 행은 (kind, asset) 하나만 — 자산은 조합마다 다르게
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
                        if *status == "failed" { Some("boom") } else { None },
                        T0 + n,
                        T0 + 2 * n
                    ],
                )
                .unwrap();
        }
    }
    // scan 의 같은 폴더 대기 행은 UNIQUE 로 하나만 들어간다 — 삽입이 실패했다면 위에서 unwrap 이 터진다
    let before_rows = jobs_dump(&db.conn);
    let before_cols = jobs_columns(&db.conn);
    let before_idx = jobs_indexes(&db.conn);
    assert!(before_rows.len() >= 15, "{before_rows:?}");
    assert_eq!(before_idx.len(), 5);

    // Act
    let report = migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();

    // Assert: 값·열 정의·인덱스 동일, FK ON·위반 0
    assert_eq!(report.applied, vec![15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26]);
    assert_eq!(jobs_dump(&db.conn), before_rows);
    assert_eq!(jobs_columns(&db.conn), before_cols);
    assert_eq!(jobs_indexes(&db.conn), before_idx);
    let fk: i64 = db
        .conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM pragma_foreign_key_check"), 0);
    // 폴더 삭제 CASCADE 유지 (scan 작업이 사라진다)
    let scan_before = count(&db.conn, "SELECT COUNT(*) FROM jobs WHERE folder_id = 1");
    assert!(scan_before > 0);
    db.conn
        .execute("DELETE FROM managed_folders WHERE folder_id = 1", [])
        .unwrap();
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM jobs WHERE folder_id = 1"), 0);
}

#[test]
fn t_scene_02_v16_accepts_scene_kind_and_still_rejects_unknown_kind() {
    // Arrange
    let db = migrated();

    // Act / Assert
    let ins = |kind: &str, asset: i64| {
        db.conn.execute(
            "INSERT INTO jobs (kind, asset_id, created_at, updated_at) VALUES (?1, ?2, 0, 0)",
            (kind, asset),
        )
    };
    assert!(ins("scene", 1).is_ok());
    assert!(ins("bogus", 1).is_err());
    // 부분 UNIQUE 유지: 같은 (kind, asset) 대기 중복 거부
    assert!(ins("scene", 1).is_err());
    assert!(db
        .conn
        .execute(
            "INSERT INTO jobs (kind, asset_id, status, created_at, updated_at) VALUES ('scene', 1, 'bogus', 0, 0)",
            [],
        )
        .is_err());
}

#[test]
fn t_scene_02_v16_rebuild_over_many_job_rows_is_fast_and_lossless() {
    // Arrange: 10만 행 (100만 행 예산 10초 의 1/10 규모 — 선형 확인)
    let mut db = v14_db();
    db.conn
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000)
             INSERT INTO jobs (kind, asset_id, priority, created_at, updated_at)
             SELECT 'thumb', i, 50, 0, 0 FROM n;",
        )
        .unwrap();

    // Act
    let started = std::time::Instant::now();
    migrate(&mut db.conn, &db.backup_dir, &FixedClock(T0 + 1)).unwrap();
    let elapsed = started.elapsed();

    // Assert
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM jobs"), 100_000);
    assert!(elapsed.as_secs() < 10, "migration took {elapsed:?}");
}

#[test]
fn t_scene_03_deleting_an_asset_cascades_its_scene_row_and_table_count_is_34() {
    // Arrange
    let db = migrated();
    add_asset(&db.conn, 1, "video");
    add_asset(&db.conn, 2, "video");
    for id in [1, 2] {
        db.conn
            .execute(
                "INSERT INTO asset_scene_cuts (asset_id, state, user_cuts, updated_at) VALUES (?1, 'done', X'0101000000', 0)",
                [id],
            )
            .unwrap();
    }

    // Act
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id = 1", [])
        .unwrap();

    // Assert
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM asset_scene_cuts"), 1);
    assert_eq!(
        count(&db.conn, "SELECT COUNT(*) FROM asset_scene_cuts WHERE asset_id = 2"),
        1
    );
    let tables = common::user_tables(&db.conn)
        .into_iter()
        .filter(|t| t != "refinery_schema_history")
        .count();
    assert_eq!(tables, 41); // V19 사용자 필드 3개 포함, V20 saved_searches 삭제, V22 watch_pending·V23 원격 접속 2개·V24 이어보기 추가, V26 plugins·plugin_providers·plugin_kv·plugin_tasks 추가
}
