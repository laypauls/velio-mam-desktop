//! 조정기 + SQLite 색인 워커 — 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 Tantivy.
//! 분류: T-coord-13 Business rule(폴더 범위 매핑·비활성 제외) / T-coord-14 Concurrency(재구축 중 검색·세대)

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::small;
use mam_db::Clock;
use mam_search::coordinator::{CoordRequest, Direction, SortSpec};
use mam_search::search::FolderFilter;
use mam_search::sync::{SyncConfig, SyncWorker};
use mam_search::{FilenameStrategy, Filters, IndexOptions, SearchError};
use rusqlite::{Connection, params};
use tempfile::{TempDir, tempdir};

struct LateClock;
impl Clock for LateClock {
    fn now_ms(&self) -> i64 {
        10_000_000
    }
}

fn setup() -> (TempDir, Connection) {
    let tmp = tempdir().unwrap();
    let mut conn = mam_db::open(&tmp.path().join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &tmp.path().join("backups"), &mam_db::SystemClock).unwrap();
    conn.execute(
        "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (1, 'fixed', 'v1', 0)",
        [],
    )
    .unwrap();
    // 폴더 1 = 'photos', 폴더 2 = 'clips'
    for (id, key) in [(1, "photos"), (2, "clips")] {
        conn.execute(
            "INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at)
             VALUES (?1, 1, ?2, ?2, ?2, 0)",
            params![id, key],
        )
        .unwrap();
    }
    (tmp, conn)
}

fn add(conn: &Connection, rel_path: &str) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 10, 0, 0, 0)",
        [],
    )
    .unwrap();
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, 1, ?2, lower(?2), 0)",
        params![id, rel_path],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, 1, 'upsert', 0, 0)",
        [id],
    )
    .unwrap();
    tx.commit().unwrap();
    id as u64
}

fn req(filters: Filters) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters,
        sort: SortSpec::default(),
        limit: 100,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

fn folder(folder_id: u64, dir: &str, recursive: bool) -> Filters {
    Filters {
        folder: Some(FolderFilter {
            folder_id,
            dir_key: dir.into(),
            recursive,
        }),
        ..Default::default()
    }
}

fn worker(tmp: &TempDir) -> SyncWorker {
    let dir = tmp.path().join("idx");
    let index =
        mam_search::SearchIndex::open_or_create(&dir, small(IndexOptions::strategy_b())).unwrap();
    SyncWorker::new(
        index,
        &dir,
        small(IndexOptions::strategy_b()),
        SyncConfig::default(),
    )
}

#[test]
fn t_coord_13_folder_scope_comes_from_managed_folders_and_inactive_assets_are_excluded() {
    // Arrange
    let (tmp, conn) = setup();
    let a = add(&conn, "Photos/2024/A.jpg");
    let b = add(&conn, "Photos/2024/Trip/B.jpg");
    let c = add(&conn, "Photos/root.jpg");
    let d = add(&conn, "Clips/d.jpg");
    let sup = add(&conn, "Photos/2024/old.jpg"); // 내용 변경으로 대체됨
    let detached = add(&conn, "Photos/2024/gone.jpg"); // 관리 폴더 제거 등으로 활성 위치 없음
    conn.execute(
        "UPDATE media_assets SET status = 'superseded', revision = 2 WHERE asset_id = ?1",
        [sup as i64],
    )
    .unwrap();
    conn.execute(
        "UPDATE file_locations SET detached_at = 1 WHERE asset_id = ?1",
        [detached as i64],
    )
    .unwrap();
    let w = worker(&tmp);
    let report = w.drain(&conn, &LateClock).unwrap();
    let h = w.search_handle();
    let ids = |f: Filters| {
        let mut v = h.search(&req(f)).unwrap().ids;
        v.sort_unstable();
        v
    };

    // Act + Assert
    assert_eq!(report.events_done, 6);
    assert_eq!(
        ids(Filters::default()),
        [a, b, c, d],
        "superseded and detached are excluded"
    );
    assert_eq!(ids(folder(1, "", true)), [a, b, c]);
    assert_eq!(ids(folder(1, "", false)), [c], "directly under the root");
    assert_eq!(ids(folder(1, "2024", true)), [a, b]);
    assert_eq!(ids(folder(1, "2024", false)), [a]);
    assert_eq!(
        ids(folder(1, "2024/trip", false)),
        [b],
        "keys are lower-case path_key form"
    );
    assert_eq!(ids(folder(2, "", true)), [d]);
}

#[test]
fn t_coord_14_searches_survive_a_rebuild_swap_and_old_cursors_become_stale() {
    // Arrange
    let (tmp, conn) = setup();
    for i in 0..300 {
        add(&conn, &format!("Photos/f{i}.jpg"));
    }
    let mut w = worker(&tmp);
    w.drain(&conn, &LateClock).unwrap();
    let h = w.search_handle();
    let mut first = req(Filters::default());
    first.limit = 10;
    let page = h.search(&first).unwrap();
    let cursor = page.next_cursor.clone().unwrap();
    let generation = h.generation();

    // 재구축 중 다른 스레드가 계속 검색한다
    let stop = Arc::new(AtomicBool::new(false));
    let ok = Arc::new(AtomicUsize::new(0));
    let reader = {
        let (h, stop, ok) = (h.clone(), Arc::clone(&stop), Arc::clone(&ok));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match h.search(&req(Filters::default())) {
                    Ok(p) => {
                        assert_eq!(p.total, 300, "a search never sees a half-swapped index");
                        ok.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(SearchError::NotReady) => {} // 교체 구간
                    Err(e) => panic!("unexpected: {e}"),
                }
            }
        })
    };

    // Act
    for _ in 0..3 {
        w.rebuild(&conn).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    // Assert
    assert!(ok.load(Ordering::Relaxed) > 0);
    assert!(
        h.generation() > generation,
        "each swap bumps the generation"
    );
    let mut stale = first.clone();
    stale.cursor = Some(cursor);
    assert!(matches!(h.search(&stale), Err(SearchError::StaleCursor)));
    assert_eq!(
        h.search(&first).unwrap().total,
        300,
        "first page works again"
    );
}

#[test]
fn t_coord_15_search_is_not_ready_instead_of_empty_while_the_first_build_runs() {
    // Arrange: 인덱스 디렉터리가 비어 있고 SQLite 에는 자산이 있다 -> open 안에서 재구축이 돈다
    let (tmp, conn) = setup();
    const N: usize = 3000;
    for i in 0..N {
        add(&conn, &format!("Photos/f{i}.jpg"));
    }
    // Outbox 가 비어 있어야 열기 단계의 문서 수 어긋남(CountDrift)으로 재구축한다 (이벤트가 있으면 워커가 차근히 채운다)
    conn.execute("DELETE FROM search_outbox", []).unwrap();
    let handle = mam_search::coordinator::SearchHandle::new();
    let stop = Arc::new(AtomicBool::new(false));
    let empty_successes = Arc::new(AtomicUsize::new(0));
    let not_ready = Arc::new(AtomicUsize::new(0));
    let reader = {
        let (h, stop, empty, nr) = (
            handle.clone(),
            Arc::clone(&stop),
            Arc::clone(&empty_successes),
            Arc::clone(&not_ready),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match h.search(&req(Filters::default())) {
                    Ok(p) if p.total == 0 => {
                        empty.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(p) => assert_eq!(p.total, N as u64, "never a half-built index"),
                    Err(SearchError::NotReady) => {
                        nr.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => panic!("unexpected: {e}"),
                }
            }
        })
    };

    // Act
    let (_worker, outcome) = SyncWorker::open_with_handle(
        &conn,
        &tmp.path().join("idx"),
        small(IndexOptions::strategy_b()),
        SyncConfig::default(),
        handle.clone(),
        &mut |_| Ok(()),
    )
    .unwrap();
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    // Assert
    assert!(outcome.rebuilt.is_some());
    assert_eq!(
        empty_successes.load(Ordering::Relaxed),
        0,
        "a build in progress must not answer with an empty success"
    );
    assert!(
        not_ready.load(Ordering::Relaxed) > 0,
        "the reader did overlap the build"
    );
    assert_eq!(
        handle.search(&req(Filters::default())).unwrap().total,
        N as u64
    );
}
