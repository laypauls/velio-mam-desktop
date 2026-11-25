//! Outbox 색인 워커 테스트 — 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 Tantivy.
//! 분류: T-sync-01 Happy / T-sync-02 Happy / T-sync-03 Concurrency(멱등) / T-sync-04·05 Business rule /
//! T-sync-06 External / T-sync-07 Business rule / T-sync-08 External / T-sync-09 Concurrency /
//! T-sync-10 Boundary / T-sync-11 Business rule / T-sync-12 External(잠금)

mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{ids, req, small};
use mam_db::Clock;
use mam_search::FilenameStrategy::A;
use mam_search::sync::{
    FULLTEXT_INDEX, RebuildReason, SyncConfig, SyncError, SyncWorker, load as sync_load,
};
use mam_search::{IndexOptions, MediaType, Presence, RetryDirectory, SearchError, SearchIndex};
use rusqlite::{Connection, params};
use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    Directory, DirectoryLock, FileHandle, FileSlice, Lock, WatchCallback, WatchHandle, WritePtr,
};
use tempfile::{TempDir, tempdir};

struct FixedClock(i64);
impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

/// 대량 이벤트 모으기 시간(2s)이 지난 시각.
const LATE: FixedClock = FixedClock(1_000_000);
const T0: i64 = 1_000;

struct Env {
    tmp: TempDir,
    conn: Connection,
}

impl Env {
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let mut conn = mam_db::open(&tmp.path().join("t.db")).unwrap();
        mam_db::migrate(&mut conn, &tmp.path().join("backups"), &mam_db::SystemClock).unwrap();
        conn.execute(
            "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (1, 'fixed', 'v1', 0)",
            [],
        )
        .unwrap();
        Self { tmp, conn }
    }

    fn db_path(&self) -> PathBuf {
        self.tmp.path().join("t.db")
    }

    fn index_dir(&self) -> PathBuf {
        self.tmp.path().join("idx")
    }

    fn other_conn(&self) -> Connection {
        mam_db::open(&self.db_path()).unwrap()
    }

    fn worker(&self) -> SyncWorker {
        self.worker_with(SyncConfig::default())
    }

    fn worker_with(&self, config: SyncConfig) -> SyncWorker {
        let index =
            SearchIndex::open_or_create(&self.index_dir(), small(IndexOptions::all())).unwrap();
        SyncWorker::new(index, &self.index_dir(), small(IndexOptions::all()), config)
    }

    fn open_worker(&self, config: SyncConfig) -> (SyncWorker, mam_search::sync::OpenOutcome) {
        SyncWorker::open(
            &self.conn,
            &self.index_dir(),
            small(IndexOptions::all()),
            config,
        )
        .unwrap()
    }
}

fn add_asset(conn: &Connection, rel_path: &str, priority: i64) -> u64 {
    add_asset_typed(conn, rel_path, "image", priority)
}

fn add_asset_typed(conn: &Connection, rel_path: &str, media_type: &str, priority: i64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES (?1, 1234, 0, 0, 0)",
        params![media_type],
    )
    .unwrap();
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, 1, ?2, lower(?2), 0)",
        params![id, rel_path],
    )
    .unwrap();
    event(&tx, id as u64, 1, "upsert", priority);
    tx.commit().unwrap();
    id as u64
}

fn event(conn: &Connection, asset_id: u64, revision: u64, kind: &str, priority: i64) -> i64 {
    conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![asset_id as i64, revision as i64, kind, priority, T0],
    )
    .unwrap();
    conn.last_insert_rowid()
}

/// 자산 변경 + revision 증가 + 이벤트를 한 트랜잭션으로 (앱의 쓰기 규칙 모사). 새 revision 반환.
fn bump(conn: &Connection, asset_id: u64, priority: i64) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    let rev = bump_in(&tx, asset_id);
    event(&tx, asset_id, rev, "upsert", priority);
    tx.commit().unwrap();
    rev
}

fn bump_in(conn: &Connection, asset_id: u64) -> u64 {
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

fn set_tags(conn: &Connection, asset_id: u64, tags: &[&str], priority: i64) -> u64 {
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

fn soft_delete(conn: &Connection, asset_id: u64) -> u64 {
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

/// (status, attempts, last_error)
fn ev(conn: &Connection, event_id: i64) -> (String, i64, Option<String>) {
    conn.query_row(
        "SELECT status, attempts, last_error FROM search_outbox WHERE event_id = ?1",
        [event_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

fn progress(conn: &Connection, asset_id: u64) -> Option<u64> {
    conn.query_row(
        "SELECT applied_revision FROM index_progress WHERE index_name = ?1 AND asset_id = ?2",
        params![FULLTEXT_INDEX, asset_id as i64],
        |r| r.get::<_, i64>(0),
    )
    .ok()
    .map(|v| v as u64)
}

fn count_status(conn: &Connection, status: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM search_outbox WHERE status = ?1",
        [status],
        |r| r.get(0),
    )
    .unwrap()
}

fn find(w: &SyncWorker, text: &str) -> Vec<u64> {
    ids(w.index().unwrap(), &req(text, A))
}

// ---------------------------------------------------------------------------------------------

#[test]
fn t_sync_01_load_doc_maps_asset_location_metadata_and_tags() {
    // Arrange
    let env = Env::new();
    let id = add_asset_typed(&env.conn, "여행/2024 제주/한글 사진.MP4", "video", 1);
    env.conn
        .execute(
            "INSERT INTO media_metadata (asset_id, duration_ms, captured_at) VALUES (?1, 5000, 1700000000123)",
            [id as i64],
        )
        .unwrap();
    set_tags(&env.conn, id, &["여행", "Beach"], 1);
    env.conn
        .execute(
            "UPDATE file_locations SET presence = 'missing' WHERE asset_id = ?1",
            [id as i64],
        )
        .unwrap();
    // 다른 자산(활성 아님·위치 분리)은 문서가 되지 않는다
    let superseded = add_asset(&env.conn, "old.jpg", 1);
    env.conn
        .execute(
            "UPDATE media_assets SET status = 'superseded' WHERE asset_id = ?1",
            [superseded as i64],
        )
        .unwrap();
    let detached = add_asset(&env.conn, "detached.jpg", 1);
    env.conn
        .execute(
            "UPDATE file_locations SET detached_at = 5 WHERE asset_id = ?1",
            [detached as i64],
        )
        .unwrap();

    // Act
    let doc = sync_load::load_doc(&env.conn, id).unwrap().unwrap();

    // Assert
    assert_eq!(doc.asset_id, id);
    assert_eq!(doc.revision, 2, "add(1) + tag edit(2)");
    assert_eq!(doc.file_name, "한글 사진.MP4");
    assert_eq!(doc.media_type, MediaType::Video);
    assert_eq!(doc.volume_id, 1);
    assert_eq!(doc.presence, Presence::Missing);
    assert_eq!(doc.file_size, 1234);
    assert_eq!(doc.captured_at, Some(1_700_000_000), "ms -> unix sec");
    assert_eq!(doc.duration_ms, Some(5000));
    let mut tags = doc.user_tags.clone();
    tags.sort();
    assert_eq!(tags, ["Beach", "여행"]);
    assert!(
        sync_load::load_doc(&env.conn, superseded)
            .unwrap()
            .is_none()
    );
    assert!(sync_load::load_doc(&env.conn, detached).unwrap().is_none());
    assert!(sync_load::load_doc(&env.conn, 99_999).unwrap().is_none());
    assert_eq!(sync_load::count_docs(&env.conn).unwrap(), 1);
}

#[test]
fn t_sync_01b_batch_queries_use_indexes_not_table_scans() {
    // Arrange
    let env = Env::new();
    for i in 0..50 {
        add_asset(&env.conn, &format!("f{i}.jpg"), 1);
    }
    env.conn.execute_batch("ANALYZE").unwrap();

    for sql in [
        sync_load::DOCS_BY_IDS_SQL,
        sync_load::REVISIONS_BY_IDS_SQL,
        sync_load::APPLIED_BY_IDS_SQL,
        mam_search::sync::SELECT_PENDING_SQL,
    ] {
        let args: Vec<&str> = if sql == mam_search::sync::SELECT_PENDING_SQL {
            vec!["10"]
        } else if sql == sync_load::APPLIED_BY_IDS_SQL {
            vec!["fulltext", "[1,2,3]"]
        } else {
            vec!["[1,2,3]"]
        };
        // Act
        let mut stmt = env
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();

        // Assert: 배열(json_each, 별칭 j; 키워드 펼침 je 는 행 1개의 JSON) 외에는 전체 스캔이 없고 정렬용 임시 B-tree 도 없다
        assert!(plan.iter().all(|l| !l.contains("TEMP B-TREE")), "{plan:#?}");
        for line in &plan {
            assert!(
                !line.starts_with("SCAN")
                    || line.starts_with("SCAN j ")
                    || line.starts_with("SCAN je "),
                "table scan in plan: {line}\nfull plan: {plan:#?}"
            );
        }
    }
}

#[test]
fn t_sync_02_upsert_event_becomes_searchable_and_is_marked_done() {
    // Arrange
    let env = Env::new();
    let w = env.worker();
    let id = add_asset(&env.conn, "trips/beach_trip.jpg", 0);
    set_tags(&env.conn, id, &["여름"], 0);

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(r.events_selected, 2);
    assert_eq!((r.upserted, r.deleted, r.events_done), (1, 0, 2));
    assert_eq!(r.error, None);
    assert_eq!(find(&w, "beach"), [id]);
    assert_eq!(find(&w, "여름"), [id], "tag from rev 2 is indexed");
    assert_eq!(progress(&env.conn, id), Some(2));
    assert_eq!(count_status(&env.conn, "done"), 2);
    assert_eq!(count_status(&env.conn, "pending"), 0);
}

#[test]
fn t_sync_03_same_event_twice_keeps_single_doc_without_error() {
    // Arrange
    let env = Env::new();
    let w = env.worker();
    let id = add_asset(&env.conn, "dup.jpg", 0);
    w.process_batch(&env.conn, &LATE).unwrap();
    let dup = event(&env.conn, id, 1, "upsert", 0); // 같은 이벤트가 중복 기록됨

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(r.error, None);
    assert_eq!((r.upserted, r.skipped_stale, r.events_done), (0, 1, 1));
    assert_eq!(w.index().unwrap().num_docs(), 1);
    assert_eq!(find(&w, "dup"), [id]);
    assert_eq!(ev(&env.conn, dup).0, "done");
}

#[test]
fn t_sync_04_late_lower_revision_event_is_ignored() {
    // Arrange: rev 1 -> rev 3 반영 후, rev 2 이벤트가 뒤늦게 도착
    let env = Env::new();
    let w = env.worker();
    let id = add_asset(&env.conn, "before.jpg", 0);
    bump(&env.conn, id, 0);
    bump(&env.conn, id, 0);
    w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!(progress(&env.conn, id), Some(3));
    // SQLite 를 revision 증가 없이 바꿔 둔다 — 낮은 revision 이벤트가 문서를 다시 읽으면 이 이름이 색인된다
    env.conn
        .execute(
            "UPDATE file_locations SET rel_path = 'after.jpg', path_key = 'after.jpg' WHERE asset_id = ?1",
            [id as i64],
        )
        .unwrap();
    let late = event(&env.conn, id, 2, "upsert", 0);

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!((r.upserted, r.skipped_stale, r.events_done), (0, 1, 1));
    assert_eq!(find(&w, "before"), [id], "index untouched by stale event");
    assert!(find(&w, "after").is_empty());
    assert_eq!(progress(&env.conn, id), Some(3), "progress never goes down");
    assert_eq!(ev(&env.conn, late).0, "done");
}

#[test]
fn t_sync_05_stale_upsert_after_delete_does_not_resurrect() {
    // Arrange
    let env = Env::new();
    let w = env.worker();
    let id = add_asset(&env.conn, "gone.jpg", 0);
    bump(&env.conn, id, 0);
    bump(&env.conn, id, 0); // rev 3
    w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!(find(&w, "gone"), [id]);
    let del_rev = soft_delete(&env.conn, id); // rev 4
    let r = w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!((r.deleted, r.events_done), (1, 1));
    assert!(find(&w, "gone").is_empty());
    assert_eq!(progress(&env.conn, id), Some(del_rev));
    // SQLite 가 (잘못되어) 활성으로 보이는 상황에서 낮은 revision upsert 가 늦게 도착
    env.conn
        .execute(
            "UPDATE media_assets SET status = 'active' WHERE asset_id = ?1",
            [id as i64],
        )
        .unwrap();
    let late = event(&env.conn, id, 2, "upsert", 0);

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert: 삭제 revision(4) 이하라 무시 — 인덱스가 되살아나지 않는다
    assert_eq!((r.upserted, r.skipped_stale), (0, 1));
    assert!(find(&w, "gone").is_empty());
    assert_eq!(w.index().unwrap().num_docs(), 0);
    assert_eq!(ev(&env.conn, late).0, "done");

    // 정당한 복원(더 높은 revision)은 반영된다
    let restored = bump(&env.conn, id, 0);
    assert!(restored > del_rev);
    w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!(find(&w, "gone"), [id]);
    assert_eq!(progress(&env.conn, id), Some(restored));
}

/// 데이터 파일(.idx/.store/.fast) `open_write` 를 `fail` 번 실패시키는 디렉터리 래퍼
/// (재시도 계층 **위** — 재시도로 복구되지 않는 지속 실패).
#[derive(Clone, Debug)]
struct FailDir {
    inner: RetryDirectory,
    fail: Arc<AtomicUsize>,
}

impl Directory for FailDir {
    fn get_file_handle(&self, p: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.inner.get_file_handle(p)
    }
    fn open_read(&self, p: &Path) -> Result<FileSlice, OpenReadError> {
        self.inner.open_read(p)
    }
    fn delete(&self, p: &Path) -> Result<(), DeleteError> {
        self.inner.delete(p)
    }
    fn exists(&self, p: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(p)
    }
    fn open_write(&self, p: &Path) -> Result<WritePtr, OpenWriteError> {
        let data_file = matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("idx" | "store" | "fast")
        );
        if data_file
            && self
                .fail
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            let e = io::Error::from(io::ErrorKind::PermissionDenied);
            return Err(OpenWriteError::wrap_io_error(e, PathBuf::from(p)));
        }
        self.inner.open_write(p)
    }
    fn atomic_read(&self, p: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.inner.atomic_read(p)
    }
    fn atomic_write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        self.inner.atomic_write(p, data)
    }
    fn sync_directory(&self) -> io::Result<()> {
        self.inner.sync_directory()
    }
    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        self.inner.acquire_lock(lock)
    }
    fn watch(&self, cb: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.inner.watch(cb)
    }
}

#[test]
fn t_sync_06_commit_failure_keeps_event_pending_then_fails_after_five_attempts() {
    // Arrange
    let env = Env::new();
    let fail = Arc::new(AtomicUsize::new(0));
    let f = fail.clone();
    let index = SearchIndex::open_or_create_with_directory(
        &env.index_dir(),
        small(IndexOptions::all()),
        move |inner| Box::new(FailDir { inner, fail: f }),
    )
    .unwrap();
    let w = SyncWorker::new(
        index,
        &env.index_dir(),
        small(IndexOptions::all()),
        SyncConfig::default(),
    );
    let id = add_asset(&env.conn, "flaky.jpg", 0);
    let ev_id: i64 = env
        .conn
        .query_row("SELECT event_id FROM search_outbox", [], |r| r.get(0))
        .unwrap();

    // Act + Assert: 1~4회 실패 -> pending 유지, attempts 증가, 오류 기록
    for attempt in 1..=4 {
        fail.store(1, Ordering::SeqCst);
        let r = w.process_batch(&env.conn, &LATE).unwrap();
        assert!(r.error.is_some(), "attempt {attempt}");
        assert_eq!((r.events_retry, r.events_failed, r.events_done), (1, 0, 0));
        assert_eq!((r.upserted, r.deleted), (0, 0), "nothing was applied");
        let (status, attempts, last_error) = ev(&env.conn, ev_id);
        assert_eq!((status.as_str(), attempts), ("pending", attempt));
        assert!(last_error.is_some_and(|e| !e.is_empty()));
        assert_eq!(progress(&env.conn, id), None, "no progress without commit");
        assert_eq!(w.index().unwrap().num_docs(), 0);
    }
    // 5회째 실패 -> failed (이력 보존), 더 이상 선택되지 않는다
    fail.store(1, Ordering::SeqCst);
    let r = w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!((r.events_retry, r.events_failed), (0, 1));
    let (status, attempts, last_error) = ev(&env.conn, ev_id);
    assert_eq!((status.as_str(), attempts), ("failed", 5));
    assert!(last_error.is_some());
    assert_eq!(
        w.process_batch(&env.conn, &LATE).unwrap().events_selected,
        0
    );

    // 새 이벤트는 실패 뒤에도 정상 처리된다 (writer 복구, 손실 없음)
    let id2 = add_asset(&env.conn, "recovered.jpg", 0);
    let r = w.process_batch(&env.conn, &LATE).unwrap();
    assert_eq!((r.error, r.upserted, r.events_done), (None, 1, 1));
    assert_eq!(find(&w, "recovered"), [id2]);
}

#[test]
fn t_sync_06b_event_succeeds_after_transient_failure_and_keeps_error_history() {
    // Arrange
    let env = Env::new();
    let fail = Arc::new(AtomicUsize::new(0));
    let f = fail.clone();
    let index = SearchIndex::open_or_create_with_directory(
        &env.index_dir(),
        small(IndexOptions::all()),
        move |inner| Box::new(FailDir { inner, fail: f }),
    )
    .unwrap();
    let w = SyncWorker::new(
        index,
        &env.index_dir(),
        small(IndexOptions::all()),
        SyncConfig::default(),
    );
    let id = add_asset(&env.conn, "retry.jpg", 0);
    fail.store(1, Ordering::SeqCst);
    assert!(w.process_batch(&env.conn, &LATE).unwrap().error.is_some());

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!((r.error, r.events_done), (None, 1));
    assert_eq!(find(&w, "retry"), [id]);
    let (status, attempts, last_error) = ev(&env.conn, 1);
    assert_eq!((status.as_str(), attempts), ("done", 1));
    assert!(last_error.is_some(), "error history is preserved");
}

#[test]
fn t_sync_07_user_edit_priority_is_applied_before_bulk_events() {
    // Arrange: 대량(priority 1) 이벤트가 먼저(낮은 event_id), 사용자 편집(0)이 나중
    let env = Env::new();
    let w = env.worker();
    let bulk = add_asset(&env.conn, "bulk.jpg", 1);
    let user = add_asset(&env.conn, "user.jpg", 0);

    // Act: 모으는 중(시각 < 생성+2s)에도 사용자 편집은 즉시
    let early = FixedClock(T0 + 10);
    let r1 = w.process_batch(&env.conn, &early).unwrap();

    // Assert
    assert_eq!((r1.upserted, r1.events_done), (1, 1));
    assert_eq!(find(&w, "user"), [user]);
    assert!(find(&w, "bulk").is_empty(), "bulk event waits");
    // 이어서 대량 이벤트: 아직 모으는 중이면 보류, 시간이 지나면 처리
    let r2 = w.process_batch(&env.conn, &early).unwrap();
    assert_eq!(r2.events_selected, 0);
    assert_eq!(r2.deferred_until_ms, Some(T0 + 2_000));
    assert!(find(&w, "bulk").is_empty());
    let r3 = w.process_batch(&env.conn, &FixedClock(T0 + 2_000)).unwrap();
    assert_eq!((r3.upserted, r3.deferred_until_ms), (1, None));
    assert_eq!(find(&w, "bulk"), [bulk]);
}

#[test]
fn t_sync_07b_full_bulk_batch_is_processed_without_waiting() {
    // Arrange: batch_size 개의 저우선 이벤트 -> 더 모을 필요 없음
    let env = Env::new();
    let w = env.worker_with(SyncConfig {
        batch_size: 3,
        ..SyncConfig::default()
    });
    for i in 0..3 {
        add_asset(&env.conn, &format!("b{i}.jpg"), 1);
    }

    // Act
    let r = w.process_batch(&env.conn, &FixedClock(T0)).unwrap();

    // Assert
    assert_eq!((r.deferred_until_ms, r.upserted), (None, 3));
}

#[test]
fn t_sync_08_corrupt_or_mismatched_index_is_rebuilt_from_sqlite() {
    // Arrange: 정상 색인 상태
    let env = Env::new();
    let mut expected = Vec::new();
    for i in 0..30 {
        expected.push(add_asset(&env.conn, &format!("clip_{i}.jpg"), 0));
    }
    soft_delete(&env.conn, expected[3]); // 삭제된 자산은 재구축 결과에도 없어야 한다
    let gone = expected.remove(3);
    {
        let w = env.worker();
        w.drain(&env.conn, &LATE).unwrap();
        assert_eq!(find(&w, "clip"), expected);
    }

    let meta = env.index_dir().join("meta.json");
    let mam_meta = env.index_dir().join("mam-search.meta");

    // Case 1: meta.json 손상
    std::fs::write(&meta, b"{ not json").unwrap();
    let (w, out) = env.open_worker(SyncConfig::default());
    let rebuilt = out.rebuilt.expect("corruption must trigger rebuild");
    assert!(
        matches!(rebuilt.reason, RebuildReason::OpenFailed(_)),
        "{rebuilt:?}"
    );
    assert_eq!(rebuilt.docs, expected.len() as u64);
    assert_eq!(find(&w, "clip"), expected, "index == SQLite");
    assert!(!find(&w, "clip").contains(&gone));
    assert_eq!(progress(&env.conn, expected[0]), Some(1));
    drop(w);

    // Case 2: 분석기/옵션 메타 불일치
    std::fs::write(
        &mam_meta,
        "format_version=1\nanalyzer_version=1\nbuild_ngram=true\nbuild_raw=true\n",
    )
    .unwrap();
    let (w, out) = env.open_worker(SyncConfig::default());
    let rebuilt = out.rebuilt.expect("mismatch must trigger rebuild");
    assert!(
        matches!(rebuilt.reason, RebuildReason::Mismatch(_)),
        "{rebuilt:?}"
    );
    assert_eq!(find(&w, "clip"), expected);
    drop(w);

    // Case 3: 정상 인덱스는 재구축하지 않는다
    let (w, out) = env.open_worker(SyncConfig::default());
    assert!(out.rebuilt.is_none());
    drop(w);

    // Case 4: 문서 수 이탈 (SQLite 에는 자산이 있는데 인덱스가 비어 있음, pending 이벤트 없음)
    std::fs::remove_dir_all(env.index_dir()).unwrap();
    let (w, out) = env.open_worker(SyncConfig::default());
    let rebuilt = out.rebuilt.expect("count drift must trigger rebuild");
    assert!(
        matches!(
            rebuilt.reason,
            RebuildReason::CountDrift {
                sqlite_docs: 29,
                index_docs: 0,
                ..
            }
        ),
        "{rebuilt:?}"
    );
    assert_eq!(find(&w, "clip"), expected);

    // 정리: 임시/이전 디렉터리가 남지 않는다
    assert!(!env.tmp.path().join("idx.rebuild").exists());
    assert!(!env.tmp.path().join("idx.old").exists());
    assert!(rebuilt.cleanup_error.is_none());
}

#[test]
fn t_sync_09_events_arriving_during_rebuild_are_applied_afterwards() {
    // Arrange: 색인된 자산 5개 (이벤트 done), 이후 인덱스를 잃은 상황
    let env = Env::new();
    let first = add_asset(&env.conn, "keep_one.jpg", 0);
    for i in 1..5 {
        add_asset(&env.conn, &format!("keep_{i}.jpg"), 0);
    }
    env.conn
        .execute("UPDATE search_outbox SET status = 'done'", [])
        .unwrap();
    let mut w = env.worker_with(SyncConfig {
        rebuild_progress_every: 1,
        ..SyncConfig::default()
    });
    let other = env.other_conn();
    let mut injected = None;

    // Act: 재구축 도중(읽기 스냅샷이 열린 뒤) 다른 연결이 새 자산 추가 + 기존 자산 태그 변경
    let report = w
        .rebuild_with(&env.conn, RebuildReason::Requested, &mut |p| {
            if p.docs_written == 2 {
                let new_id = add_asset(&other, "late_arrival.jpg", 0);
                let rev = set_tags(&other, first, &["새태그"], 0);
                injected = Some((new_id, rev));
            }
        })
        .unwrap();
    let (new_id, edited_rev) = injected.expect("callback ran mid-rebuild");

    // Assert: 재구축 직후에는 스냅샷 내용만, 이벤트는 pending 으로 남아 있다
    assert_eq!(report.docs, 5);
    assert!(find(&w, "late_arrival").is_empty());
    assert!(find(&w, "새태그").is_empty());
    assert_eq!(count_status(&env.conn, "pending"), 2);
    assert_eq!(progress(&env.conn, first), Some(1));

    // 이후 처리: 새 자산·변경이 반영된다 (멱등, 역전 없음)
    let r = w.drain(&env.conn, &LATE).unwrap();
    assert_eq!((r.error, r.events_done), (None, 2));
    assert_eq!(find(&w, "late_arrival"), [new_id]);
    assert_eq!(find(&w, "새태그"), [first]);
    assert_eq!(progress(&env.conn, first), Some(edited_rev));
    assert_eq!(w.index().unwrap().num_docs(), 6);
    assert_eq!(count_status(&env.conn, "pending"), 0);
}

#[test]
fn t_sync_10_empty_outbox_is_a_noop() {
    // Arrange
    let env = Env::new();
    let w = env.worker();

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();
    let d = w.drain(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(r, Default::default());
    assert_eq!(d, Default::default());
    assert_eq!(w.index().unwrap().num_docs(), 0);
}

#[test]
fn t_sync_11_event_for_missing_or_detached_asset_removes_it_from_index() {
    // Arrange: 색인된 자산 2개
    let env = Env::new();
    let w = env.worker();
    let hard = add_asset(&env.conn, "hard_deleted.jpg", 0);
    let detached = add_asset(&env.conn, "detached_loc.jpg", 0);
    w.drain(&env.conn, &LATE).unwrap();
    assert_eq!(w.index().unwrap().num_docs(), 2);
    // 행이 사라진 자산(outbox/progress 는 FK 가 없어 남는다)과 활성 위치가 없어진 자산
    env.conn
        .execute(
            "DELETE FROM file_locations WHERE asset_id = ?1",
            [hard as i64],
        )
        .unwrap();
    env.conn
        .execute(
            "DELETE FROM media_assets WHERE asset_id = ?1",
            [hard as i64],
        )
        .unwrap();
    event(&env.conn, hard, 2, "delete", 0);
    env.conn
        .execute(
            "UPDATE file_locations SET detached_at = 1 WHERE asset_id = ?1",
            [detached as i64],
        )
        .unwrap();
    bump(&env.conn, detached, 0);

    // Act
    let r = w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!((r.deleted, r.events_done, r.error), (2, 2, None));
    assert_eq!(w.index().unwrap().num_docs(), 0);
    assert_eq!(
        progress(&env.conn, hard),
        Some(2),
        "delete revision recorded"
    );
}

#[test]
fn t_sync_12_locked_index_is_not_rebuilt_or_destroyed() {
    // Arrange: 다른 SyncWorker 가 이미 인덱스 쓰기 잠금을 쥐고 있다
    let env = Env::new();
    add_asset(&env.conn, "locked.jpg", 0);
    let w = env.worker();
    w.drain(&env.conn, &LATE).unwrap();

    // Act
    let second = SyncWorker::open(
        &env.conn,
        &env.index_dir(),
        small(IndexOptions::all()),
        SyncConfig::default(),
    );

    // Assert: 오류를 반환하고 기존 인덱스는 그대로
    assert!(
        matches!(
            second,
            Err(SyncError::Search(SearchError::Tantivy(
                tantivy::TantivyError::LockFailure(..)
            )))
        ),
        "{:?}",
        second.err()
    );
    assert_eq!(find(&w, "locked").len(), 1);
    assert!(!env.tmp.path().join("idx.rebuild").exists());
}

#[test]
fn t_sync_13_user_data_is_loaded_into_the_doc_and_memo_is_searchable() {
    // Arrange: 메모·즐겨찾기·컬렉션·태그가 있는 자산과 아무것도 없는 자산
    let env = Env::new();
    let w = env.worker();
    let rich = add_asset(&env.conn, "plain_a.jpg", 0);
    let plain = add_asset(&env.conn, "plain_b.jpg", 0);
    set_tags(&env.conn, rich, &["여행"], 0);
    env.conn
        .execute_batch(&format!(
            "INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES ({rich}, '제주도 일출 사진', 1, 0);
             INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (4, 'c4', 'c4', 0), (6, 'c6', 'c6', 0);
             INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES (4, {rich}, 0), (6, {rich}, 0);"
        ))
        .unwrap();
    bump(&env.conn, rich, 0);

    // Act
    let doc = sync_load::load_doc(&env.conn, rich).unwrap().unwrap();
    let blank = sync_load::load_doc(&env.conn, plain).unwrap().unwrap();
    w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(doc.user_memo, "제주도 일출 사진");
    assert!(doc.favorite);
    let mut cols = doc.collection_ids.clone();
    cols.sort();
    assert_eq!(cols, [4, 6]);
    assert_eq!(doc.tag_ids.len(), 1);
    assert!(!blank.favorite);
    assert!(
        blank.user_memo.is_empty() && blank.collection_ids.is_empty() && blank.tag_ids.is_empty()
    );
    assert_eq!(find(&w, "일출"), [rich], "memo is indexed");
}

#[test]
fn t_sync_14_index_built_before_user_data_fields_is_rebuilt() {
    // Arrange: 이전 분석기 버전(v2)으로 만든 인덱스
    let env = Env::new();
    let id = add_asset(&env.conn, "old_index.jpg", 0);
    {
        let w = env.worker();
        w.drain(&env.conn, &LATE).unwrap();
    }
    std::fs::write(
        env.index_dir().join("mam-search.meta"),
        "format_version=1\nanalyzer_version=2\nbuild_ngram=true\nbuild_raw=true\n",
    )
    .unwrap();

    // Act
    let (w, out) = env.open_worker(SyncConfig::default());

    // Assert
    let rebuilt = out.rebuilt.expect("v2 index must be rebuilt");
    assert!(
        matches!(rebuilt.reason, RebuildReason::Mismatch(_)),
        "{rebuilt:?}"
    );
    assert_eq!(find(&w, "old_index"), [id]);
}

/// V6 AI 결과 한 벌 (분석 + 전사) 을 자산에 붙인다.
fn add_ai_rows(conn: &Connection, id: u64, transcript: &str) {
    conn.execute(
        "INSERT INTO ai_analyses (asset_id, description, keywords, ocr_text, summary, language, provider, model,
                                  analyzed_at, updated_at)
         VALUES (?1, '해변에서 노는 강아지', '[\"강아지\",\"해변\"]', 'OPEN SIGN', '여름 휴가 요약', 'ko', 'openai', 'm', 0, 0)",
        params![id as i64],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO transcripts (asset_id, language, text, engine, model, created_at)
         VALUES (?1, 'ko', ?2, 'whisper.cpp', 'm', 0)",
        params![id as i64, transcript],
    )
    .unwrap();
}

// 분류: Happy / Boundary(전사 상한·AI 행 없음) / Business rule(행 1건 = 문서 1건)
#[test]
fn t_sync_15_ai_analysis_and_transcript_are_loaded_into_the_doc_and_searchable() {
    // Arrange: AI 결과가 있는 자산과 없는 자산, 상한을 넘는 전사
    let env = Env::new();
    let w = env.worker();
    let rich = add_asset(&env.conn, "plain_a.jpg", 0);
    let plain = add_asset(&env.conn, "plain_b.jpg", 0);
    let long = format!("{} 마지막꼬리", "가".repeat(200_000));
    add_ai_rows(&env.conn, rich, &long);
    bump(&env.conn, rich, 0);

    // Act
    let doc = sync_load::load_doc(&env.conn, rich).unwrap().unwrap();
    let blank = sync_load::load_doc(&env.conn, plain).unwrap().unwrap();
    let all = sync_load::load_docs(&env.conn, &[rich, plain]).unwrap();
    w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(all.len(), 2, "LEFT JOIN must not duplicate or drop assets");
    assert_eq!(
        doc.ai_description,
        "해변에서 노는 강아지\nOPEN SIGN\n여름 휴가 요약"
    );
    assert_eq!(doc.ai_keywords, ["강아지", "해변"]);
    assert_eq!(doc.transcript.chars().count(), 200_000, "capped");
    assert!(!doc.transcript.contains("마지막꼬리"));
    assert!(blank.ai_description.is_empty() && blank.ai_keywords.is_empty());
    assert!(blank.transcript.is_empty());
    assert_eq!(find(&w, "강아지"), [rich], "keyword/description indexed");
    assert_eq!(find(&w, "휴가"), [rich], "summary indexed");
}

// 분류: Boundary(손상된 키워드 JSON 은 스키마 CHECK 로 차단) / Business rule(성능: 인덱스 사용)
#[test]
fn t_sync_16_doc_queries_with_ai_joins_use_indexes_not_table_scans() {
    // Arrange
    let env = Env::new();
    for i in 0..50 {
        let id = add_asset(&env.conn, &format!("f{i}.jpg"), 1);
        add_ai_rows(&env.conn, id, "전사");
    }
    env.conn.execute_batch("ANALYZE").unwrap();

    for sql in [sync_load::DOCS_BY_IDS_SQL, sync_load::ALL_DOCS_SQL] {
        // Act
        let mut stmt = env
            .conn
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let args: Vec<&str> = if sql == sync_load::DOCS_BY_IDS_SQL {
            vec!["[1,2,3]"]
        } else {
            vec![]
        };
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();

        // Assert: ai_analyses / transcripts 는 키 검색이어야 한다 (SCAN 은 구동 테이블·json_each 만)
        let joined = plan.join("\n");
        assert!(
            joined.contains("SEARCH ai ") && joined.contains("SEARCH tr "),
            "{plan:#?}"
        );
        for line in &plan {
            assert!(
                !(line.starts_with("SCAN ai") || line.starts_with("SCAN tr")),
                "scan on joined AI table: {line}\n{plan:#?}"
            );
        }
    }
}

#[test]
fn t_sync_17_title_is_loaded_from_sqlite_and_searchable_like_a_file_name() {
    // Arrange: 제목이 있는 자산 / 제목이 NULL 인 자산 / 공백뿐인 제목(비정상 저장값)
    let env = Env::new();
    let w = env.worker();
    let titled = add_asset(&env.conn, "IMG_0001.jpg", 0);
    let plain = add_asset(&env.conn, "IMG_0002.jpg", 0);
    env.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, title, updated_at) VALUES (?1, '제주 일출 여행', 0)",
            [titled as i64],
        )
        .unwrap();
    bump(&env.conn, titled, 0);

    // Act
    let doc = sync_load::load_doc(&env.conn, titled).unwrap().unwrap();
    let blank = sync_load::load_doc(&env.conn, plain).unwrap().unwrap();
    w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(doc.title, "제주 일출 여행");
    assert!(blank.title.is_empty());
    let find_title = |w: &SyncWorker, text: &str| {
        let mut r = req(text, A);
        r.filters.search_title = true;
        ids(w.index().unwrap(), &r)
    };
    assert_eq!(find_title(&w, "일출"), [titled], "title is indexed");
    assert!(find(&w, "일출").is_empty(), "title search is gated off by default");
    assert_eq!(find(&w, "img_0001").len(), 1, "file name is still searchable");

    // 제목을 지우면(NULL) 같은 revision 규칙으로 다시 색인되어 더 이상 찾지 못한다
    env.conn
        .execute(
            "UPDATE asset_user_data SET title = NULL WHERE asset_id = ?1",
            [titled as i64],
        )
        .unwrap();
    bump(&env.conn, titled, 0);
    w.process_batch(&env.conn, &LATE).unwrap();
    assert!(find_title(&w, "일출").is_empty());
}

#[test]
fn t_sync_18_index_built_before_titles_existed_is_rebuilt_and_finds_titles() {
    // Arrange: 제목 필드 이전 분석기 버전(v5)으로 만든 인덱스 + 그 뒤 저장된 제목
    let env = Env::new();
    let id = add_asset(&env.conn, "old_index.jpg", 0);
    {
        let w = env.worker();
        w.drain(&env.conn, &LATE).unwrap();
    }
    std::fs::write(
        env.index_dir().join("mam-search.meta"),
        "format_version=1\nanalyzer_version=5\nbuild_ngram=true\nbuild_raw=true\n",
    )
    .unwrap();
    env.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, title, updated_at) VALUES (?1, '노을 풍경', 0)",
            [id as i64],
        )
        .unwrap();

    // Act
    let (w, out) = env.open_worker(SyncConfig::default());

    // Assert
    let rebuilt = out.rebuilt.expect("v5 index must be rebuilt");
    assert!(
        matches!(rebuilt.reason, RebuildReason::Mismatch(_)),
        "{rebuilt:?}"
    );
    let mut r = req("노을", A);
    r.filters.search_title = true;
    assert_eq!(ids(w.index().unwrap(), &r), [id]);
}

// 분류: Happy (rating 적재·재색인) / Boundary (행 없음 = 0) / Business rule (revision 갱신으로 반영)
#[test]
fn t_search_rating_08_rating_is_loaded_into_the_doc_and_a_change_reindexes_the_asset() {
    // Arrange: 별점 4 행이 있는 자산과 행이 없는 자산
    let env = Env::new();
    let w = env.worker();
    let rated = add_asset(&env.conn, "plain_a.jpg", 0);
    let plain = add_asset(&env.conn, "plain_b.jpg", 0);
    env.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, rating, updated_at) VALUES (?1, 4, 0)",
            [rated as i64],
        )
        .unwrap();
    bump(&env.conn, rated, 0);

    // Act
    let doc = sync_load::load_doc(&env.conn, rated).unwrap().unwrap();
    let blank = sync_load::load_doc(&env.conn, plain).unwrap().unwrap();
    w.process_batch(&env.conn, &LATE).unwrap();
    let in_range = |min, max| {
        let mut r = req("plain", A);
        r.filters.rating = mam_search::Range { min, max };
        ids(w.index().unwrap(), &r)
    };

    // Assert
    assert_eq!((doc.rating, blank.rating), (4, 0));
    assert_eq!(in_range(Some(4), Some(4)), [rated]);
    assert_eq!(in_range(Some(0), Some(0)), [plain], "no row = unrated");

    // Act: 별점을 0 으로 내리고 revision 을 올리면 색인이 따라간다
    env.conn
        .execute(
            "UPDATE asset_user_data SET rating = 0 WHERE asset_id = ?1",
            [rated as i64],
        )
        .unwrap();
    bump(&env.conn, rated, 0);
    w.process_batch(&env.conn, &LATE).unwrap();

    // Assert
    assert_eq!(in_range(Some(1), None), Vec::<u64>::new());
    assert_eq!(in_range(Some(0), Some(0)), {
        let mut v = vec![rated, plain];
        v.sort_unstable();
        v
    });
}

#[test]
fn t_search_rating_09_index_built_before_the_rating_field_is_rebuilt() {
    // Arrange: 별점 필드가 없던 분석기 버전(v7)으로 만든 인덱스 + 별점이 있는 자산
    let env = Env::new();
    let id = add_asset(&env.conn, "old_index.jpg", 0);
    env.conn
        .execute(
            "INSERT INTO asset_user_data (asset_id, rating, updated_at) VALUES (?1, 3, 0)",
            [id as i64],
        )
        .unwrap();
    bump(&env.conn, id, 0);
    {
        let w = env.worker();
        w.drain(&env.conn, &LATE).unwrap();
    }
    std::fs::write(
        env.index_dir().join("mam-search.meta"),
        "format_version=1\nanalyzer_version=7\nbuild_ngram=true\nbuild_raw=true\n",
    )
    .unwrap();

    // Act
    let (w, out) = env.open_worker(SyncConfig::default());

    // Assert: 재구축되고 새 필드가 채워진다
    let rebuilt = out.rebuilt.expect("v7 index must be rebuilt");
    assert!(
        matches!(rebuilt.reason, RebuildReason::Mismatch(_)),
        "{rebuilt:?}"
    );
    let mut r = req("old_index", A);
    r.filters.rating = mam_search::Range { min: Some(3), max: Some(3) };
    assert_eq!(ids(w.index().unwrap(), &r), [id]);
}
