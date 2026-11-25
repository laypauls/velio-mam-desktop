//! 복원·복구 시험 공용 헬퍼 — 실제 SQLite 임시 파일, 주입 시계. 사용자 폴더는 건드리지 않는다.
#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use mam_core::storage::DataPaths;
use mam_db::{Clock, settings};
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

use crate::startup::{Boot, ProcessLauncher, boot};

pub const T0: i64 = 1_800_000_000_000;

pub struct TestClock(pub AtomicI64);

impl TestClock {
    pub fn new() -> Self {
        Self(AtomicI64::new(T0))
    }
    pub fn set(&self, ms: i64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 레이아웃 + 마이그레이션이 끝난 MAMData (연결은 모두 닫힌 상태)
pub fn data_dir() -> (TempDir, DataPaths) {
    let dir = TempDir::new().unwrap();
    let paths = DataPaths::new(dir.path().join("MAMData"));
    paths.ensure_layout().unwrap();
    let mut conn = mam_db::open(&paths.db()).unwrap();
    mam_db::migrate(&mut conn, &paths.backups(), &TestClock::new()).unwrap();
    (dir, paths)
}

pub fn conn(paths: &DataPaths) -> Connection {
    mam_db::open(&paths.db()).unwrap()
}

pub fn insert_asset(conn: &Connection) -> i64 {
    conn.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 10, 1, 1, 1)",
        [],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn count(paths: &DataPaths, sql: &str) -> i64 {
    conn(paths).query_row(sql, [], |r| r.get(0)).unwrap()
}

pub fn set_consent(conn: &Connection, given: bool, at: Option<i64>) {
    let doc = json!({"settings": {"consent": {"given": given, "at": at}}});
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('ai_settings', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [doc.to_string()],
    )
    .unwrap();
}

pub fn consent(conn: &Connection) -> (bool, Option<i64>) {
    let raw: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'ai_settings'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    (
        doc["settings"]["consent"]["given"].as_bool().unwrap(),
        doc["settings"]["consent"]["at"].as_i64(),
    )
}

pub fn set_hash(conn: &mut Connection, hash: &str) {
    assert!(settings::swap_password_hash(conn, None, hash).unwrap());
}

pub fn hash(paths: &DataPaths) -> Option<String> {
    settings::load_password_hash(&conn(paths)).unwrap()
}

/// 전체 기동 경로(journal -> 초기화/복원 표지 -> 열기·복구·마이그레이션)를 실제로 돈다.
pub fn run_boot(paths: &DataPaths, clock: &dyn Clock) -> Boot {
    boot(&[], "0.1.0", paths, clock, &ProcessLauncher)
}

/// 손상 시험: 전체를 쓰레기 바이트로 (`SQLITE_NOTADB`)
pub fn overwrite_with_garbage(paths: &DataPaths) {
    std::fs::write(paths.db(), vec![0xAB; 16 * 1024]).unwrap();
}
