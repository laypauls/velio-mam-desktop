//! P4 복원력 — 비정상 종료·Outbox 재실행 (T-res-01 ~ 04). 실제 SQLite(마이그레이션) + 임시 디렉터리 Tantivy.
//! 분류: T-res-01 Concurrency(원자성) / T-res-02 External(commit 전 종료) / T-res-03 Concurrency(멱등·재개) /
//! T-res-04 Business rule(역전 방지). 원본 보호: 시나리오 끝에 픽스처 원본 불변 단언.

mod common;
#[path = "common/resilience.rs"]
mod res;

use mam_search::sync::{RebuildReason, SyncConfig, load as sync_load};
use mam_search::{SearchIndex, UpsertOutcome};
use res::*;

const NAMES: [&str; 6] = [
    "alpha.jpg",
    "bravo.jpg",
    "charlie.jpg",
    "delta.jpg",
    "echo.jpg",
    "foxtrot.jpg",
];

fn seed(env: &Env, n: usize, priority: i64) -> Vec<u64> {
    NAMES[..n]
        .iter()
        .map(|name| add_asset(&env.conn, 1, name, priority))
        .collect()
}

#[test]
fn t_res_01_connection_dropped_before_commit_leaves_neither_asset_nor_event() {
    // Arrange: 커밋된 자산 1건 + 다른 연결에서 커밋하지 않은 자산·이벤트
    let env = Env::new();
    let originals = Originals::new(&NAMES);
    let kept = add_asset(&env.conn, 1, "kept.jpg", 0);
    {
        let doomed = env.reopen_conn();
        let tx = doomed.unchecked_transaction().unwrap();
        insert_asset_in(&tx, 1, "doomed.jpg", 0);
        // 같은 트랜잭션 안에서는 보이지만 커밋 전이다
        assert_eq!(count(&tx, "SELECT count(*) FROM search_outbox"), 2);
        assert_eq!(count(&env.conn, "SELECT count(*) FROM search_outbox"), 1);
        std::mem::forget(tx); // 롤백 정리 없이 연결만 죽는다 (프로세스 종료 모사)
        drop(doomed);
    }

    // Act: 재시작 — 새 연결로 열어 일관성 확인 후 워커 구동
    let conn = env.reopen_conn();
    let w = env.worker();
    let r = w.drain(&conn, &LATE).unwrap();

    // Assert: 자산과 이벤트가 함께 없다 (원자성), 무결성 검사 통과
    assert_eq!(count(&conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM file_locations"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM search_outbox"), 1);
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    assert_eq!((r.upserted, r.events_done), (1, 1));
    assert_eq!(all_indexed_ids(w.index().unwrap()), [kept]);
    assert!(find(w.index().unwrap(), "doomed").is_empty());
    originals.assert_untouched();
}

#[test]
fn t_res_01b_rollback_after_asset_insert_before_event_insert_is_all_or_nothing() {
    // Arrange
    let env = Env::new();
    let before = (
        count(&env.conn, "SELECT count(*) FROM media_assets"),
        count(&env.conn, "SELECT count(*) FROM search_outbox"),
    );

    // Act: 자산·위치까지 쓰고 이벤트 기록 단계에서 실패(제약 위반) -> 트랜잭션 폐기
    let tx = env.conn.unchecked_transaction().unwrap();
    env.conn
        .execute(
            "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
             VALUES ('image', 1, 0, 0, 0)",
            [],
        )
        .unwrap();
    let bad_event = env.conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (1, 0, 'upsert', 0, 0)", // revision >= 1 CHECK 위반
        [],
    );
    drop(tx); // 롤백

    // Assert
    assert!(bad_event.is_err(), "CHECK(revision >= 1) must reject");
    assert_eq!(
        (
            count(&env.conn, "SELECT count(*) FROM media_assets"),
            count(&env.conn, "SELECT count(*) FROM search_outbox")
        ),
        before
    );
}

#[test]
fn t_res_02_writer_dropped_before_commit_is_recovered_by_outbox_replay() {
    // Arrange: 자산 5건(이벤트 pending). 워커가 문서를 writer 에 넣었지만 commit 전에 종료
    let env = Env::new();
    let originals = Originals::new(&NAMES);
    let ids = seed(&env, 5, 0);
    {
        let index = SearchIndex::open_or_create(&env.index_dir(), opts()).unwrap();
        let docs = sync_load::load_docs(&env.conn, &ids).unwrap();
        for id in &ids {
            assert!(matches!(
                index.upsert(&docs[id]).unwrap(),
                UpsertOutcome::Applied
            ));
        }
        assert_eq!(index.num_docs(), 0, "nothing is committed yet");
    } // commit 없이 drop = 프로세스 종료

    // Act: 재시작 (재구축이 아니라 outbox 재처리로 따라잡아야 한다)
    let (w, out) = env.start(&env.conn, SyncConfig::default());
    assert!(out.rebuilt.is_none(), "pending events cover the gap");
    assert_eq!(count_status(&env.conn, "pending"), 5, "no event was acked");
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert: 인덱스 문서 = SQLite 활성 자산
    assert_eq!((r.upserted, r.events_done, r.error), (5, 5, None));
    assert_eq!(
        all_indexed_ids(w.index().unwrap()),
        sqlite_active_ids(&env.conn)
    );
    assert_eq!(w.index().unwrap().num_docs(), 5);
    for id in &ids {
        assert_eq!(progress(&env.conn, *id), Some(1));
    }
    originals.assert_untouched();
}

#[test]
fn t_res_02b_crash_after_index_commit_before_ack_replays_without_duplicates() {
    // Arrange: 인덱스 commit 은 끝났지만 SQLite 에 done/progress 를 쓰기 전에 종료.
    // 그 사이 한 자산은 삭제되어 delete 이벤트가 pending 이다.
    let env = Env::new();
    let originals = Originals::new(&NAMES);
    let ids = seed(&env, 4, 0);
    {
        let index = SearchIndex::open_or_create(&env.index_dir(), opts()).unwrap();
        let docs = sync_load::load_docs(&env.conn, &ids).unwrap();
        for id in &ids {
            index.upsert(&docs[id]).unwrap();
        }
        index.commit().unwrap();
    }
    let gone = ids[1];
    soft_delete(&env.conn, gone);
    assert_eq!(count_status(&env.conn, "done"), 0);

    // Act
    let (w, out) = env.start(&env.conn, SyncConfig::default());
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert: 중복 문서 없음, 삭제 반영, 모든 이벤트 완료
    assert!(out.rebuilt.is_none());
    assert_eq!(r.error, None);
    assert_eq!(count_status(&env.conn, "pending"), 0);
    assert_eq!(w.index().unwrap().num_docs(), 3);
    let expect: Vec<u64> = ids.iter().copied().filter(|i| *i != gone).collect();
    assert_eq!(all_indexed_ids(w.index().unwrap()), expect);
    assert_eq!(
        all_indexed_ids(w.index().unwrap()),
        sqlite_active_ids(&env.conn)
    );
    for id in &expect {
        assert_eq!(
            find(
                w.index().unwrap(),
                NAMES[(*id - 1) as usize].trim_end_matches(".jpg")
            ),
            [*id]
        );
    }
    assert_eq!(progress(&env.conn, gone), Some(2));
    originals.assert_untouched();
}

#[test]
fn t_res_03_interrupted_partial_processing_resumes_idempotently_with_monotonic_progress() {
    // Arrange: 6건, 배치 크기 2 — 첫 워커가 배치 1개만 처리하고 죽는다
    let env = Env::new();
    let originals = Originals::new(&NAMES);
    let cfg = SyncConfig {
        batch_size: 2,
        ..SyncConfig::default()
    };
    let ids = seed(&env, 6, 0);
    let snapshot_progress = |conn: &rusqlite::Connection| -> Vec<Option<u64>> {
        ids.iter().map(|i| progress(conn, *i)).collect()
    };
    {
        let w = env.worker_with(cfg.clone());
        let r = w.process_batch(&env.conn, &LATE).unwrap();
        assert_eq!((r.events_selected, r.upserted), (2, 2));
    }
    let after_first = snapshot_progress(&env.conn);
    assert_eq!(count_status(&env.conn, "pending"), 4);

    // Act 1: 재시작 후 한 배치 더 + 그 동안 일부 자산에 새 revision
    let conn = env.reopen_conn();
    let (w, out) = env.start(&conn, cfg.clone());
    assert!(out.rebuilt.is_none());
    let r2 = w.process_batch(&conn, &LATE).unwrap();
    assert_eq!(r2.events_selected, 2);
    bump(&conn, ids[0], 0); // 이미 처리된 자산 재편집 -> rev 2
    let after_second = snapshot_progress(&conn);
    drop(w);

    // Act 2: 또 재시작해 남은 것을 모두 처리
    let (w, _) = env.start(&conn, cfg.clone());
    let rest = w.drain(&conn, &LATE).unwrap();
    let after_all = snapshot_progress(&conn);

    // Assert: 단조 증가, 중복 문서 없음, 인덱스 = SQLite
    for (a, b) in [(&after_first, &after_second), (&after_second, &after_all)] {
        for (x, y) in a.iter().zip(b) {
            assert!(x.unwrap_or(0) <= y.unwrap_or(0), "{a:?} -> {b:?}");
        }
    }
    assert_eq!(rest.error, None);
    assert_eq!(w.index().unwrap().num_docs(), 6);
    assert_eq!(all_indexed_ids(w.index().unwrap()), ids);
    assert_eq!(after_all[0], Some(2));
    assert!(after_all[1..].iter().all(|p| *p == Some(1)));
    assert_eq!(count_status(&conn, "pending"), 0);

    // Act 3: 완료된 이벤트가 전부 되살아난 상황(재전송·중복) -> 인덱스·progress 불변
    conn.execute("UPDATE search_outbox SET status = 'pending'", [])
        .unwrap();
    let replay = w.drain(&conn, &LATE).unwrap();
    assert_eq!(replay.upserted + replay.deleted, 0, "{replay:?}");
    assert_eq!(replay.events_done, 7, "6 initial + 1 re-edit event");
    assert_eq!(snapshot_progress(&conn), after_all);
    assert_eq!(w.index().unwrap().num_docs(), 6);
    assert_eq!(count_status(&conn, "pending"), 0);
    originals.assert_untouched();
}

#[test]
fn t_res_04_stale_events_after_restart_and_rebuild_never_revert_or_resurrect() {
    // Arrange: rev 3 까지 반영 후 삭제(rev 4) 반영, 같은 배치 안에서 순서가 뒤섞인 이벤트도 준비
    let env = Env::new();
    let originals = Originals::new(&NAMES);
    let live = add_asset(&env.conn, 1, "live_v1.jpg", 0);
    let dead = add_asset(&env.conn, 1, "dead.jpg", 0);
    bump(&env.conn, live, 0);
    bump(&env.conn, live, 0);
    {
        let w = env.worker();
        w.drain(&env.conn, &LATE).unwrap();
        assert_eq!(progress(&env.conn, live), Some(3));
    }
    let del_rev = soft_delete(&env.conn, dead);
    let (mut w, _) = env.start(&env.conn, SyncConfig::default());
    w.drain(&env.conn, &LATE).unwrap();
    assert_eq!(progress(&env.conn, dead), Some(del_rev));

    // Act 1: 재시작 후 낮은 revision 이벤트가 (순서 뒤바뀐 채) 도착. SQLite 는 revision 증가 없이 바뀐 상태.
    env.conn
        .execute(
            "UPDATE file_locations SET rel_path = 'live_v2.jpg', path_key = 'live_v2.jpg' WHERE asset_id = ?1",
            [live as i64],
        )
        .unwrap();
    env.conn
        .execute(
            "UPDATE media_assets SET status = 'active' WHERE asset_id = ?1",
            [dead as i64],
        )
        .unwrap();
    event(&env.conn, live, 3, "upsert", 0); // 같은 revision 재전송
    event(&env.conn, live, 2, "upsert", 0); // 더 낮음 — 나중에 도착
    event(&env.conn, dead, 2, "upsert", 0); // 삭제 이전 revision
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert 1: 인덱스 불변, progress 불변
    assert_eq!(r.upserted + r.deleted, 0, "{r:?}");
    assert_eq!(find(w.index().unwrap(), "live_v1"), [live]);
    assert!(find(w.index().unwrap(), "live_v2").is_empty());
    assert!(find(w.index().unwrap(), "dead").is_empty());
    assert_eq!(progress(&env.conn, live), Some(3));
    assert_eq!(progress(&env.conn, dead), Some(del_rev));

    env.conn
        .execute(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = ?1",
            [dead as i64],
        )
        .unwrap(); // 삭제 상태를 SQLite 에 되돌려 둔다 (재구축은 SQLite 가 기준)

    // Act 2: 전체 재구축 뒤에도 낮은 revision 이벤트는 인덱스를 되돌리지 못한다
    // (재구축은 SQLite 현재 상태 = live_v2 를 반영하고 progress 를 스냅샷 revision 으로 덮어쓴다)
    let rep = w.rebuild(&env.conn).unwrap();
    assert_eq!(rep.reason, RebuildReason::Requested);
    assert_eq!(find(w.index().unwrap(), "live_v2"), [live]);
    env.conn
        .execute(
            "UPDATE file_locations SET rel_path = 'live_v3.jpg', path_key = 'live_v3.jpg' WHERE asset_id = ?1",
            [live as i64],
        )
        .unwrap();
    event(&env.conn, live, 1, "upsert", 0);
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert 2
    assert_eq!(r.upserted, 0, "{r:?}");
    assert_eq!(find(w.index().unwrap(), "live_v2"), [live]);
    assert!(find(w.index().unwrap(), "live_v3").is_empty());
    assert_eq!(progress(&env.conn, live), Some(3));

    // 정당한 갱신(더 높은 revision)은 반영된다
    let rev = bump(&env.conn, live, 0);
    w.drain(&env.conn, &LATE).unwrap();
    assert_eq!(find(w.index().unwrap(), "live_v3"), [live]);
    assert_eq!(progress(&env.conn, live), Some(rev));
    originals.assert_untouched();
}
