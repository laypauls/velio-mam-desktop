//! 통합 테스트 공용 헬퍼 — 실제 SQLite 임시 파일에 0001 을 적용해 사용한다.
#![allow(dead_code)] // 테스트 파일마다 쓰는 헬퍼가 다르다

use std::path::PathBuf;

use mam_db::{Clock, Migration, migrate, open};
use rusqlite::Connection;
use tempfile::TempDir;

/// 고정 시각 — 백업 파일명 결정성을 위해 실제 시계를 읽지 않는다.
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

pub const T0: i64 = 1_700_000_000_000;

pub struct TestDb {
    pub dir: TempDir,
    pub db_path: PathBuf,
    pub backup_dir: PathBuf,
    pub conn: Connection,
}

/// 마이그레이션이 적용된 빈 DB.
pub fn migrated_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    migrate(&mut conn, &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

/// 마이그레이션 전 빈 DB (연결 PRAGMA 만 설정됨).
pub fn empty_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let conn = open(&db_path).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

pub fn user_tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub fn applied_versions(conn: &Connection) -> Vec<i64> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'refinery_schema_history')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    if !exists {
        return Vec::new();
    }
    let mut stmt = conn
        .prepare("SELECT version FROM refinery_schema_history ORDER BY version")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub fn dir_entries(path: &std::path::Path) -> Vec<String> {
    match std::fs::read_dir(path) {
        Ok(rd) => rd
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

// --- 행 삽입 헬퍼 -------------------------------------------------------

pub fn insert_volume(conn: &Connection, key: &str) -> i64 {
    conn.execute(
        "INSERT INTO volumes (kind, volume_key, last_seen_at) VALUES ('fixed', ?1, ?2)",
        (key, T0),
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// 해시 미계산(pending) 이미지 자산.
pub fn insert_asset(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 10, ?1, ?1, ?1)",
        [T0],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn insert_location(
    conn: &Connection,
    asset_id: i64,
    volume_id: i64,
    path_key: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, ?2, ?3, ?3, ?4)",
        (asset_id, volume_id, path_key, T0),
    )
}

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

// --- 마이그레이션 픽스처 ------------------------------------------------

pub const V1_SQL: &str = include_str!("../../migrations/V1__core_schema.sql");
pub const V2_SQL: &str = "CREATE TABLE future_table (a INTEGER PRIMARY KEY) STRICT;";

pub fn v1() -> Migration {
    Migration::unapplied("V1__core_schema", V1_SQL).unwrap()
}

pub fn v2() -> Migration {
    Migration::unapplied("V2__future", V2_SQL).unwrap()
}

pub fn schema_dump(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT type || ':' || name || ':' || IFNULL(sql, '') FROM sqlite_master ORDER BY type, name")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// V1 만 적용된 DB — "V1 시점 앱"에서 새 마이그레이션이 들어오는 시나리오용 (D2: 최신 버전 하드코딩 금지).
pub fn v1_db() -> TestDb {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("mam.sqlite");
    let backup_dir = dir.path().join("backups");
    let mut conn = open(&db_path).unwrap();
    mam_db::migrate_with(&mut conn, &[v1()], &backup_dir, &FixedClock(T0)).unwrap();
    TestDb {
        dir,
        db_path,
        backup_dir,
        conn,
    }
}

/// 앱이 아는 최신보다 하나 높은, 미래 버전 마이그레이션 (구버전 앱 롤백 시나리오).
pub fn future_migration() -> Migration {
    Migration::unapplied(
        &format!("V{}__future", mam_db::latest_version() + 1),
        V2_SQL,
    )
    .unwrap()
}

/// 앱에 포함된 마이그레이션 파일 전체 (`migrations/` 디렉터리를 읽는다 — 새 마이그레이션이 늘어도 테스트가 따라간다).
pub fn shipped_migrations() -> Vec<Migration> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort_by_key(|p| {
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        name[1..name.find("__").unwrap()].parse::<u32>().unwrap()
    });
    files
        .iter()
        .map(|p| {
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            Migration::unapplied(&name, &std::fs::read_to_string(p).unwrap()).unwrap()
        })
        .collect()
}

// --- jobs 표 비교 (재생성 마이그레이션 시험) -----------------------------

/// 공백을 하나로 접은 SQL (정의 비교용)
pub fn norm_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// jobs 의 열 정의 (이름·타입·NOT NULL·기본값·PK 순서)
pub fn jobs_columns(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT cid || ':' || name || ':' || type || ':' || \"notnull\" || ':' || IFNULL(dflt_value, '-') || ':' || pk FROM pragma_table_info('jobs') ORDER BY cid")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// jobs 인덱스 (이름 -> 공백 정규화한 정의)
pub fn jobs_indexes(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT name, sql FROM sqlite_master WHERE type = 'index' AND tbl_name = 'jobs' AND sql IS NOT NULL ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, norm_sql(&r.get::<_, String>(1)?))))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

pub fn jobs_dump(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT job_id || '|' || kind || '|' || IFNULL(asset_id, 'N') || '|' || IFNULL(folder_id, 'N') || '|' || priority || '|' || status || '|' || attempts || '|' || not_before || '|' || IFNULL(last_error, 'N') || '|' || created_at || '|' || updated_at FROM jobs ORDER BY job_id")
        .unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// --- 플러그인 정수 키 시드 (테스트는 문자열 ID 로 쓰고 레지스트리가 정수 키를 준다) -----------------

/// `plugin_tasks` 행 하나를 레지스트리를 거쳐 넣는다 (`done_revision` 은 0)
pub fn seed_task(conn: &Connection, plugin: &str, provider: &str, asset_id: i64, revision: i64, status: &str) {
    seed_task_done(conn, plugin, provider, asset_id, revision, 0, status);
}

pub fn seed_task_done(conn: &Connection, plugin: &str, provider: &str, asset_id: i64, revision: i64, done: i64, status: &str) {
    let pr = mam_db::plugin_registry::ensure(conn, plugin).unwrap();
    let vr = mam_db::plugin_registry::ensure_provider(conn, pr, provider).unwrap();
    conn.execute(
        "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, done_revision, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        (pr, vr, asset_id, revision, done, status),
    )
    .unwrap();
}

/// `plugin_kv` 행 하나 (`asset_id` `None` = 전역)
pub fn seed_kv(conn: &Connection, plugin: &str, asset_id: Option<i64>, key: &str) {
    let pr = mam_db::plugin_registry::ensure(conn, plugin).unwrap();
    conn.execute("INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, ?2, ?3, '1', 0)", (pr, asset_id, key)).unwrap();
}
