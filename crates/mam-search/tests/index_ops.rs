//! 색인 작업 테스트.
//! 분류: T-fts-05 Happy / T-fts-06 Concurrency(멱등) / T-fts-07 Business rule /
//!       T-fts-09 External failure

mod common;

use common::*;
use mam_search::FilenameStrategy::A;
use mam_search::{IndexOptions, SearchError, SearchIndex, UpsertOutcome};
use tempfile::tempdir;

#[test]
fn t_fts_05_add_commit_search_returns_asset_id() {
    let dir = tempdir().unwrap();
    let idx = open_all(&dir);
    idx.upsert(&doc(42, "IMG_4821.jpg")).unwrap();

    // commit 전에는 검색에 보이지 않는다
    assert!(ids(&idx, &req("IMG_4821.jpg", A)).is_empty());
    idx.commit().unwrap();

    let hits = idx.search(&req("IMG_4821.jpg", A)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].asset_id, 42);
    assert!(hits[0].score > 0.0);
    assert_eq!(idx.num_docs(), 1);
    // 다른 확장자·다른 번호는 일치하지 않는다
    assert!(ids(&idx, &req("IMG_4821.png", A)).is_empty());
    assert!(ids(&idx, &req("IMG_4822.jpg", A)).is_empty());
}

#[test]
fn t_fts_06_upsert_same_asset_twice_keeps_one_doc() {
    let dir = tempdir().unwrap();
    let idx = open_all(&dir);
    // 같은 배치 안 두 번 + commit 뒤 한 번 더 (같은 revision = 중복 이벤트)
    for _ in 0..2 {
        assert_eq!(
            idx.upsert(&doc(1, "first.jpg")).unwrap(),
            UpsertOutcome::Applied
        );
    }
    idx.commit().unwrap();
    assert_eq!(
        idx.upsert(&doc(1, "first.jpg")).unwrap(),
        UpsertOutcome::Applied
    );
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 1);

    // 내용이 바뀐 새 revision 은 문서를 교체한다 (옛 파일명은 사라진다)
    let mut d = doc(1, "second.jpg");
    d.revision = 2;
    assert_eq!(idx.upsert(&d).unwrap(), UpsertOutcome::Applied);
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 1);
    assert_eq!(ids(&idx, &req("second", A)), [1]);
    assert!(ids(&idx, &req("first", A)).is_empty());
}

#[test]
fn t_fts_07_lower_revision_does_not_overwrite() {
    let dir = tempdir().unwrap();
    let idx = open_all(&dir);
    let mut newer = doc(1, "new_name.jpg");
    newer.revision = 5;
    idx.upsert(&newer).unwrap();

    // 같은 배치(commit 전)의 낮은 revision
    let mut older = doc(1, "old_name.jpg");
    older.revision = 3;
    assert_eq!(
        idx.upsert(&older).unwrap(),
        UpsertOutcome::Stale {
            indexed_revision: 5
        }
    );
    idx.commit().unwrap();

    // commit 된 뒤의 낮은 revision
    older.revision = 4;
    assert_eq!(
        idx.upsert(&older).unwrap(),
        UpsertOutcome::Stale {
            indexed_revision: 5
        }
    );
    idx.commit().unwrap();

    assert_eq!(idx.num_docs(), 1);
    assert_eq!(ids(&idx, &req("new_name.jpg", A)), [1]);
    assert!(ids(&idx, &req("old_name.jpg", A)).is_empty());

    // 재시작(재오픈) 뒤에도 가드가 유지된다
    drop(idx);
    let idx =
        SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::all())).unwrap();
    assert_eq!(
        idx.upsert(&older).unwrap(),
        UpsertOutcome::Stale {
            indexed_revision: 5
        }
    );
}

#[test]
fn t_fts_07b_invalid_revision_and_known_gap_delete_then_stale_upsert() {
    let dir = tempdir().unwrap();
    let idx = open_all(&dir);
    let mut zero = doc(1, "a.jpg");
    zero.revision = 0;
    assert!(matches!(
        idx.upsert(&zero),
        Err(SearchError::InvalidInput(_))
    ));

    let mut v5 = doc(2, "five.jpg");
    v5.revision = 5;
    idx.upsert(&v5).unwrap();
    idx.commit().unwrap();
    idx.delete(2).unwrap();
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 0);
    // 알려진 한계(tombstone 없음 — 결정): 삭제 후에는 인덱스에 revision 이 남지 않아 낮은 revision
    // 의 늦은 upsert 가 되살린다. 삭제를 가로지르는 역전 방지의 정본은 SQLite
    // `index_progress.applied_revision` 이고 P2 색인 워커가 적용한다. 이 단언은 현재 동작의 기록이다.
    let mut v3 = doc(2, "three.jpg");
    v3.revision = 3;
    assert_eq!(idx.upsert(&v3).unwrap(), UpsertOutcome::Applied);
    // 같은 배치의 delete → 낮은 revision upsert 도 같은 이유로 허용된다
    idx.delete(2).unwrap();
    assert_eq!(
        idx.upsert(&doc(2, "one.jpg")).unwrap(),
        UpsertOutcome::Applied
    );
    idx.commit().unwrap();
    assert_eq!(ids(&idx, &req("one", A)), [2]);
    assert_eq!(idx.num_docs(), 1);
}

#[test]
fn t_fts_09_corrupt_or_locked_index_returns_error_then_rebuild_succeeds() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("idx");

    // 잠금 중복: 첫 핸들이 쓰기 잠금을 쥔 동안 두 번째 open 은 오류
    let first = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
    first.upsert(&doc(1, "a.jpg")).unwrap();
    first.commit().unwrap();
    let second = SearchIndex::open_or_create(&path, small(IndexOptions::all()));
    assert!(
        matches!(
            second,
            Err(SearchError::Tantivy(tantivy::TantivyError::LockFailure(..)))
        ),
        "lock must fail with LockFailure"
    );
    drop(first);

    // 메타 손상
    std::fs::write(path.join("meta.json"), b"{ not json").unwrap();
    assert!(SearchIndex::open_or_create(&path, small(IndexOptions::all())).is_err());

    // 빈 meta.json (쓰기 도중 중단 등)도 오류
    std::fs::write(path.join("meta.json"), b"").unwrap();
    assert!(SearchIndex::open_or_create(&path, small(IndexOptions::all())).is_err());

    // 삭제 후 재구축 성공
    std::fs::remove_dir_all(&path).unwrap();
    let rebuilt = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
    rebuilt.upsert(&doc(1, "a.jpg")).unwrap();
    rebuilt.commit().unwrap();
    assert_eq!(ids(&rebuilt, &req("a.jpg", A)), [1]);
}

#[test]
fn t_fts_09b_truncated_segment_file_is_an_error_not_a_panic() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("idx");
    {
        let idx = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
        idx.upsert(&doc(1, "a.jpg")).unwrap();
        idx.commit().unwrap();
    }
    // 세그먼트 데이터 파일(.idx / .term)을 비운다
    for e in std::fs::read_dir(&path).unwrap() {
        let p = e.unwrap().path();
        if matches!(p.extension().and_then(|x| x.to_str()), Some("idx" | "term")) {
            std::fs::write(&p, b"").unwrap();
        }
    }
    let outcome = std::panic::catch_unwind(|| {
        match SearchIndex::open_or_create(&path, small(IndexOptions::all())) {
            Err(_) => true,
            Ok(idx) => idx.search(&req("a.jpg", A)).is_err(),
        }
    });
    assert_eq!(
        outcome.ok(),
        Some(true),
        "must return an error without panicking"
    );
}

#[test]
fn t_fts_06b_concurrent_upserts_of_same_asset_keep_max_revision() {
    // Concurrency: 여러 스레드가 같은 asset 을 서로 다른 revision 으로 upsert
    let dir = tempdir().unwrap();
    let idx = std::sync::Arc::new(open_all(&dir));
    let threads: Vec<_> = (0..8u64)
        .map(|t| {
            let idx = idx.clone();
            std::thread::spawn(move || {
                for k in 0..25u64 {
                    let rev = 1 + t * 25 + k; // 1..=200, 스레드마다 겹치지 않음
                    let mut d = doc(7, &format!("rev{rev}.jpg"));
                    d.revision = rev;
                    idx.upsert(&d).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 1);
    assert_eq!(ids(&idx, &req("rev200", A)), [7]);
    // 더 낮은 revision 은 이제 Stale
    let mut d = doc(7, "late.jpg");
    d.revision = 150;
    assert_eq!(
        idx.upsert(&d).unwrap(),
        UpsertOutcome::Stale {
            indexed_revision: 200
        }
    );
}

#[test]
fn t_fts_06c_uncommitted_batch_is_lost_on_reopen_and_older_revision_applies() {
    // 계약: commit 전에 중단되면 그 배치는 사라지고 가드는 commit 된 revision 만 안다.
    // 따라서 재시작 후 낮은 revision(3) 이 Applied — 호출자(워커)는 commit 이 끝난 revision 만
    // SQLite `index_progress.applied_revision` 에 기록해야 한다.
    let dir = tempdir().unwrap();
    let path = dir.path().join("idx");
    {
        let idx = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
        let mut d = doc(1, "committed.jpg");
        d.revision = 2;
        idx.upsert(&d).unwrap();
        idx.commit().unwrap();
        d.revision = 5;
        d.file_name = "lost.jpg".into();
        idx.upsert(&d).unwrap(); // commit 없이 drop
    }
    let idx = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
    assert_eq!(ids(&idx, &req("committed", A)), [1]);
    assert!(ids(&idx, &req("lost", A)).is_empty());
    let mut d = doc(1, "three.jpg");
    d.revision = 3;
    assert_eq!(idx.upsert(&d).unwrap(), UpsertOutcome::Applied);
}
