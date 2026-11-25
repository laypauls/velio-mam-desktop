//! P4 복원력 시험 공용 헬퍼 — 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 Tantivy + 주입 시계.
//! `#[path = "common/resilience.rs"] mod res;` 로 포함한다 (기존 `common/mod.rs` 는 건드리지 않는다).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use mam_db::Clock;
use mam_search::sync::{FULLTEXT_INDEX, SyncConfig, SyncWorker};
use mam_search::{FilenameStrategy, IndexOptions, SearchIndex, SearchRequest};
use rusqlite::{Connection, params};
use tempfile::{TempDir, tempdir};

pub struct FixedClock(pub i64);
impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// 대량 이벤트 모으기 시간(2s)이 지난 시각.
pub const LATE: FixedClock = FixedClock(1_000_000);
pub const T0: i64 = 1_000;

pub fn small(mut o: IndexOptions) -> IndexOptions {
    o.writer_heap_bytes = 15_000_000;
    o.writer_threads = 1;
    o
}

pub fn opts() -> IndexOptions {
    small(IndexOptions::all())
}

pub struct Env {
    pub tmp: TempDir,
    pub conn: Connection,
}

impl Env {
    /// 마이그레이션이 적용된 SQLite + 볼륨 1(`v1`), 2(`v2`).
    pub fn new() -> Self {
        let tmp = tempdir().unwrap();
        let mut conn = mam_db::open(&tmp.path().join("t.db")).unwrap();
        mam_db::migrate(&mut conn, &tmp.path().join("backups"), &mam_db::SystemClock).unwrap();
        for (id, key) in [(1, "v1"), (2, "v2")] {
            conn.execute(
                "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (?1, 'removable', ?2, 0)",
                params![id, key],
            )
            .unwrap();
        }
        Self { tmp, conn }
    }

    pub fn db_path(&self) -> PathBuf {
        self.tmp.path().join("t.db")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.tmp.path().join("idx")
    }

    /// 같은 DB 파일에 대한 새 연결 (재시작 후 재오픈 모사).
    pub fn reopen_conn(&self) -> Connection {
        mam_db::open(&self.db_path()).unwrap()
    }

    /// 손상 검사 없이 인덱스를 열어 워커를 만든다 (인덱스가 비어 있는 새 시작).
    pub fn worker_with(&self, config: SyncConfig) -> SyncWorker {
        let index = SearchIndex::open_or_create(&self.index_dir(), opts()).unwrap();
        SyncWorker::new(index, &self.index_dir(), opts(), config)
    }

    pub fn worker(&self) -> SyncWorker {
        self.worker_with(SyncConfig::default())
    }

    /// 재시작: `SyncWorker::open` (손상·불일치·문서 수 이탈이면 재구축).
    pub fn start(
        &self,
        conn: &Connection,
        config: SyncConfig,
    ) -> (SyncWorker, mam_search::sync::OpenOutcome) {
        SyncWorker::open(conn, &self.index_dir(), opts(), config).unwrap()
    }
}

pub fn add_asset(conn: &Connection, vol: i64, rel_path: &str, priority: i64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    let id = insert_asset_in(&tx, vol, rel_path, priority);
    tx.commit().unwrap();
    id
}

/// 트랜잭션 안에서 자산 + 위치 + outbox 를 만든다 (커밋은 호출자 — 커밋 전 중단 모사용).
pub fn insert_asset_in(conn: &Connection, vol: i64, rel_path: &str, priority: i64) -> u64 {
    conn.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 1234, 0, 0, 0)",
        [],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, ?2, ?3, lower(?3), 0)",
        params![id, vol, rel_path],
    )
    .unwrap();
    event(conn, id as u64, 1, "upsert", priority);
    id as u64
}

pub fn event(conn: &Connection, asset_id: u64, revision: u64, kind: &str, priority: i64) -> i64 {
    conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![asset_id as i64, revision as i64, kind, priority, T0],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn bump_in(conn: &Connection, asset_id: u64) -> u64 {
    conn.execute(
        "UPDATE media_assets SET revision = revision + 1 WHERE asset_id = ?1",
        [asset_id as i64],
    )
    .unwrap();
    conn.query_row(
        "SELECT revision FROM media_assets WHERE asset_id = ?1",
        [asset_id as i64],
        |r| r.get::<_, i64>(0),
    )
    .unwrap() as u64
}

/// 자산 변경 + revision 증가 + 이벤트를 한 트랜잭션으로. 새 revision 반환.
pub fn bump(conn: &Connection, asset_id: u64, priority: i64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    let rev = bump_in(&tx, asset_id);
    event(&tx, asset_id, rev, "upsert", priority);
    tx.commit().unwrap();
    rev
}

/// 앱의 사용자 태그 편집 규칙 모사: 태그 교체 + revision + 고우선(0) 이벤트가 한 트랜잭션.
pub fn set_tags(conn: &Connection, asset_id: u64, tags: &[&str], priority: i64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "DELETE FROM asset_tags WHERE asset_id = ?1",
        [asset_id as i64],
    )
    .unwrap();
    for t in tags {
        tx.execute(
            "INSERT OR IGNORE INTO tags (name, name_key) VALUES (?1, lower(?1))",
            [t],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at)
             SELECT ?1, tag_id, 0 FROM tags WHERE name_key = lower(?2)",
            params![asset_id as i64, t],
        )
        .unwrap();
    }
    let rev = bump_in(&tx, asset_id);
    event(&tx, asset_id, rev, "upsert", priority);
    tx.commit().unwrap();
    rev
}

pub fn soft_delete(conn: &Connection, asset_id: u64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "UPDATE media_assets SET status = 'deleted' WHERE asset_id = ?1",
        [asset_id as i64],
    )
    .unwrap();
    let rev = bump_in(&tx, asset_id);
    event(&tx, asset_id, rev, "delete", 0);
    tx.commit().unwrap();
    rev
}

pub fn progress(conn: &Connection, asset_id: u64) -> Option<u64> {
    conn.query_row(
        "SELECT applied_revision FROM index_progress WHERE index_name = ?1 AND asset_id = ?2",
        params![FULLTEXT_INDEX, asset_id as i64],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .map(|v| v as u64)
}

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

pub fn count_status(conn: &Connection, status: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM search_outbox WHERE status = ?1",
        [status],
        |r| r.get(0),
    )
    .unwrap()
}

pub fn req(text: &str) -> SearchRequest {
    SearchRequest {
        text: text.to_string(),
        strategy: FilenameStrategy::A,
        limit: 1000,
        ..Default::default()
    }
}

/// 정렬된 asset_id 목록 (순서 무관 비교용).
pub fn find(index: &SearchIndex, text: &str) -> Vec<u64> {
    ids_of(index, &req(text))
}

pub fn ids_of(index: &SearchIndex, r: &SearchRequest) -> Vec<u64> {
    let mut v: Vec<u64> = index
        .search(r)
        .unwrap()
        .iter()
        .map(|h| h.asset_id)
        .collect();
    v.sort_unstable();
    v
}

/// SQLite 가 색인 대상으로 보는 활성 자산 id (정렬).
pub fn sqlite_active_ids(conn: &Connection) -> Vec<u64> {
    let mut stmt = conn
        .prepare(
            "SELECT a.asset_id FROM media_assets a
             JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
             WHERE a.status = 'active' ORDER BY a.asset_id",
        )
        .unwrap();
    stmt.query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|r| r.unwrap() as u64)
        .collect()
}

/// 인덱스 전체 문서의 id (필터 없이 전수 조회, 정렬).
pub fn all_indexed_ids(index: &SearchIndex) -> Vec<u64> {
    ids_of(index, &req(""))
}

/// 디렉터리의 (이름, 크기, 내용) 목록 — 원본 보호 가드용.
pub fn tree_snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>, std::time::SystemTime)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                let m = std::fs::metadata(&p).unwrap().modified().unwrap();
                out.push((p.clone(), std::fs::read(&p).unwrap(), m));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

type Snap = Vec<(PathBuf, Vec<u8>, std::time::SystemTime)>;

/// 원본 미디어 픽스처 (임시 디렉터리). 시나리오 끝에 `assert_untouched` 로 내용·수정 시각 불변을 확인한다.
pub struct Originals {
    dir: TempDir,
    before: Snap,
}

impl Originals {
    pub fn new(names: &[&str]) -> Self {
        let dir = tempdir().unwrap();
        for (i, n) in names.iter().enumerate() {
            std::fs::write(dir.path().join(n), vec![i as u8 + 1; 64]).unwrap();
        }
        let before = tree_snapshot(dir.path());
        Self { dir, before }
    }

    pub fn assert_untouched(&self) {
        assert_eq!(
            self.before,
            tree_snapshot(self.dir.path()),
            "original fixtures must not change"
        );
    }
}
