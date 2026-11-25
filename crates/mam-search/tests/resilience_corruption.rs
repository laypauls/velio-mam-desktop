//! P4 복원력 — Tantivy 인덱스 손상 감지·재구축 (T-res-05). 실제 SQLite + 임시 디렉터리 Tantivy.
//! 분류: External(파일 절단·쓰레기 바이트·meta 삭제) / Business rule(SQLite 가 원천 — 재구축 후 검색 결과 동일) /
//! Concurrency(재구축 직후 새 이벤트 처리). 원본 보호: 시나리오 끝에 픽스처 원본 불변 단언.

mod common;
#[path = "common/resilience.rs"]
mod res;

use std::path::{Path, PathBuf};

use mam_search::sync::{RebuildReason, SyncConfig};
use mam_search::{Filters, MediaType, SearchIndex, SearchRequest, Sort};
use res::*;

/// 세그먼트 데이터 파일 확장자 (Tantivy 0.27).
const SEGMENT_EXTS: [&str; 6] = ["idx", "term", "pos", "store", "fast", "fieldnorm"];

/// 검색 질의 묶음 — 파일명·태그·필터·전체 조회를 포함한다.
fn queries() -> Vec<SearchRequest> {
    let mut v: Vec<SearchRequest> = ["clip", "clip_07", "여행", "summer", "beach"]
        .iter()
        .map(|t| req(t))
        .collect();
    v.push(req("")); // 전체
    v.push(SearchRequest {
        filters: Filters {
            media_types: vec![MediaType::Image],
            online_volume_ids: Some(vec![1]),
            ..Default::default()
        },
        sort: Sort::CapturedAtDesc,
        limit: 1000,
        ..Default::default()
    });
    v
}

fn results(index: &SearchIndex) -> Vec<Vec<u64>> {
    queries().iter().map(|q| ids_of(index, q)).collect()
}

/// 자산 40건(일부 태그, 1건 삭제)을 색인한 환경. 반환: (환경, 손상 전 검색 결과)
fn indexed_env() -> (Env, Vec<Vec<u64>>) {
    let env = Env::new();
    let mut ids = Vec::new();
    for i in 0..40 {
        ids.push(add_asset(
            &env.conn,
            1 + (i % 2),
            &format!("clip_{i:02}.jpg"),
            0,
        ));
    }
    for id in ids.iter().step_by(4) {
        set_tags(&env.conn, *id, &["여행", "Summer Beach"], 0);
    }
    soft_delete(&env.conn, ids[5]);
    let w = env.worker();
    w.drain(&env.conn, &LATE).unwrap();
    let before = results(w.index().unwrap());
    assert_eq!(before[0].len(), 39, "deleted asset is not indexed");
    assert!(!before[2].is_empty() && !before[3].is_empty());
    assert!(!before[0].contains(&ids[5]));
    (env, before)
}

fn segment_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| SEGMENT_EXTS.contains(&e))
        })
        .collect();
    v.sort();
    assert!(!v.is_empty(), "committed index must have segment files");
    v
}

fn rewrite(p: &Path, f: impl FnOnce(&mut Vec<u8>)) {
    let mut b = std::fs::read(p).unwrap();
    f(&mut b);
    std::fs::write(p, b).unwrap();
}

type Corrupt = fn(&Path);

/// (이름, 손상 함수). 어떤 손상이든 "열기 시점에 감지 -> 재구축" 이어야 한다.
fn corruptions() -> Vec<(&'static str, Corrupt)> {
    vec![
        ("meta_json_truncated_half", |d| {
            rewrite(&d.join("meta.json"), |b| b.truncate(b.len() / 2));
        }),
        ("meta_json_deleted", |d| {
            std::fs::remove_file(d.join("meta.json")).unwrap();
        }),
        ("mam_meta_garbage", |d| {
            std::fs::write(d.join("mam-search.meta"), b"not a meta file\n").unwrap();
        }),
        ("mam_meta_deleted", |d| {
            std::fs::remove_file(d.join("mam-search.meta")).unwrap();
        }),
        ("segments_truncated_half", |d| {
            for p in segment_files(d) {
                rewrite(&p, |b| b.truncate(b.len() / 2));
            }
        }),
        ("segments_truncated_to_zero", |d| {
            for p in segment_files(d) {
                rewrite(&p, |b| b.clear());
            }
        }),
        ("segments_garbage_bytes_same_length", |d| {
            for p in segment_files(d) {
                rewrite(&p, |b| {
                    let n = b.len();
                    if n > 40 {
                        b[4..n - 24].iter_mut().for_each(|x| *x ^= 0xA5);
                    }
                });
            }
        }),
        ("one_term_file_garbage_body", |d| {
            let p = segment_files(d)
                .into_iter()
                .find(|p| p.extension().is_some_and(|e| e == "term"))
                .unwrap();
            rewrite(&p, |b| {
                let n = b.len();
                b[4..n - 24].iter_mut().for_each(|x| *x ^= 0xFF);
            });
        }),
        ("one_store_file_deleted", |d| {
            let p = segment_files(d)
                .into_iter()
                .find(|p| p.extension().is_some_and(|e| e == "store"))
                .unwrap();
            std::fs::remove_file(p).unwrap();
        }),
        ("index_dir_removed", |d| {
            std::fs::remove_dir_all(d).unwrap();
        }),
    ]
}

#[test]
fn t_res_05_corrupted_tantivy_index_is_detected_on_open_and_rebuilt_with_identical_results() {
    for (name, corrupt) in corruptions() {
        // Arrange: 정상 색인 + 손상 전 검색 결과 기록
        let (env, before) = indexed_env();
        let originals = Originals::new(&["a.jpg", "b.jpg"]);
        let sqlite_docs = sqlite_active_ids(&env.conn);
        corrupt(&env.index_dir());

        // Act: 재시작(열기)
        let (w, out) = env.start(&env.conn, SyncConfig::default());

        // Assert: 열기 시점에 감지되어 재구축됨
        let rebuilt = out
            .rebuilt
            .unwrap_or_else(|| panic!("{name}: corruption was not detected on open"));
        assert!(
            matches!(
                rebuilt.reason,
                RebuildReason::OpenFailed(_)
                    | RebuildReason::Mismatch(_)
                    | RebuildReason::CountDrift { .. }
            ),
            "{name}: {:?}",
            rebuilt.reason
        );
        assert_eq!(rebuilt.docs, sqlite_docs.len() as u64, "{name}");
        assert!(rebuilt.cleanup_error.is_none(), "{name}");
        // 검색 결과가 손상 전과 동일 (전수 비교: 전체 조회 + 질의 묶음)
        assert_eq!(results(w.index().unwrap()), before, "{name}");
        assert_eq!(all_indexed_ids(w.index().unwrap()), sqlite_docs, "{name}");
        // 재구축 부산물이 남지 않는다
        assert!(!env.tmp.path().join("idx.rebuild").exists(), "{name}");
        assert!(!env.tmp.path().join("idx.old").exists(), "{name}");
        // progress 가 스냅샷 revision 으로 복구되어 이후 정상 이벤트가 처리된다
        let first = sqlite_docs[0];
        let rev = bump(&env.conn, first, 0);
        let r = w.drain(&env.conn, &LATE).unwrap();
        assert_eq!((r.error, r.upserted), (None, 1), "{name}");
        assert_eq!(progress(&env.conn, first), Some(rev), "{name}");
        drop(w);
        // 다음 시작에서는 정상 인덱스라 재구축하지 않는다
        let (_w2, out2) = env.start(&env.conn, SyncConfig::default());
        assert!(out2.rebuilt.is_none(), "{name}: rebuilt again");
        originals.assert_untouched();
    }
}

#[test]
fn t_res_05b_corruption_with_pending_events_rebuilds_then_applies_newer_changes() {
    // Arrange: 손상된 인덱스 + 손상 이후에 쌓인 pending 이벤트(새 자산 1건, 기존 자산 편집 1건, 삭제 1건)
    let (env, _) = indexed_env();
    let ids = sqlite_active_ids(&env.conn);
    segment_files(&env.index_dir())
        .iter()
        .for_each(|p| rewrite(p, |b| b.truncate(b.len() / 2)));
    let added = add_asset(&env.conn, 1, "fresh_after_crash.jpg", 0);
    let edited = ids[1];
    let rev = set_tags(&env.conn, edited, &["after crash tag"], 0);
    let removed = ids[2];
    soft_delete(&env.conn, removed);

    // Act
    let (w, out) = env.start(&env.conn, SyncConfig::default());
    let rebuilt = out.rebuilt.expect("must rebuild");
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert: 재구축은 SQLite 최신 상태를 담고, 남은 이벤트는 멱등하게 소진된다
    assert!(matches!(rebuilt.reason, RebuildReason::OpenFailed(_)));
    assert_eq!(r.error, None);
    assert_eq!(count_status(&env.conn, "pending"), 0);
    assert_eq!(
        all_indexed_ids(w.index().unwrap()),
        sqlite_active_ids(&env.conn)
    );
    assert_eq!(find(w.index().unwrap(), "fresh_after_crash"), [added]);
    assert_eq!(find(w.index().unwrap(), "tag"), [edited]);
    assert!(!find(w.index().unwrap(), "clip").contains(&removed));
    assert_eq!(progress(&env.conn, edited), Some(rev));
}

/// 실제 결함 재현: `mam-search.meta` 가 UTF-8 이 아닌 바이트로 손상되면 `meta::check_or_write` 의
/// `read_to_string` 이 `io::ErrorKind::InvalidData` 를 `SearchError::Io` 로 반환하고,
/// `SyncWorker::open` 은 `Io` 를 재구축 사유로 보지 않아(`other => return Err`) 열기가 영구 실패한다.
/// 기대: 다른 손상(meta.json·세그먼트)처럼 감지 -> 재구축.
#[test]
fn t_res_05c_non_utf8_mam_meta_is_rebuilt_not_a_permanent_open_failure() {
    // Arrange
    let (env, before) = indexed_env();
    std::fs::write(env.index_dir().join("mam-search.meta"), b"\x00\xFF\xFE\x80").unwrap();

    // Act
    let opened = mam_search::sync::SyncWorker::open(
        &env.conn,
        &env.index_dir(),
        opts(),
        SyncConfig::default(),
    );

    // Assert
    let (w, out) = opened.expect("corrupted meta must be recoverable by rebuild");
    assert!(out.rebuilt.is_some());
    assert_eq!(results(w.index().unwrap()), before);
}
