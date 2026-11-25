//! 필터·정렬·페이지네이션·랭킹 테스트.
//! 분류: T-fts-08 Business rule / T-fts-10 Happy / 그 외 Boundary

mod common;

use common::*;
use mam_search::FilenameStrategy::A;
use mam_search::{Filters, MediaType, Presence, Range, SearchRequest, Sort};
use tempfile::tempdir;

fn with(id: u64, name: &str, f: impl FnOnce(&mut mam_search::AssetDoc)) -> mam_search::AssetDoc {
    let mut d = doc(id, name);
    f(&mut d);
    d
}

#[test]
fn t_fts_08_volume_filter_excludes_offline_volumes() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "trip.jpg", |d| d.volume_id = 1),
            with(2, "trip.jpg", |d| d.volume_id = 2),
            with(3, "trip.jpg", |d| d.volume_id = 3),
        ],
    );
    let mut r = req("trip", A);
    // 필터 없음 = 전부
    assert_eq!(ids(&idx, &r), [1, 2, 3]);
    // 온라인 볼륨 {1, 3}
    r.filters.online_volume_ids = Some(vec![1, 3]);
    assert_eq!(ids(&idx, &r), [1, 3]);
    // 온라인 볼륨이 없으면 결과 없음 (필터 없음으로 해석하지 않는다)
    r.filters.online_volume_ids = Some(vec![]);
    assert!(ids(&idx, &r).is_empty());
    // 텍스트 없이 필터만으로도 마찬가지
    let only_filter = SearchRequest {
        filters: Filters {
            online_volume_ids: Some(vec![]),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(ids(&idx, &only_filter).is_empty());
    // 존재하지 않는 볼륨 id
    r.filters.online_volume_ids = Some(vec![99]);
    assert!(ids(&idx, &r).is_empty());
}

#[test]
fn t_fts_08b_media_type_and_presence_filters() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "sea.jpg", |_| {}),
            with(2, "sea.mp4", |d| d.media_type = MediaType::Video),
            with(3, "sea.wav", |d| {
                d.media_type = MediaType::Audio;
                d.presence = Presence::Missing;
            }),
        ],
    );
    let mut r = req("sea", A);
    r.filters.media_types = vec![MediaType::Video];
    assert_eq!(ids(&idx, &r), [2]);
    r.filters.media_types = vec![MediaType::Image, MediaType::Audio];
    assert_eq!(ids(&idx, &r), [1, 3]);
    r.filters.presence = Some(Presence::Present);
    assert_eq!(ids(&idx, &r), [1]);
}

#[test]
fn t_fts_10_range_filters_and_captured_at_sort() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.mp4", |d| {
                d.captured_at = Some(100);
                d.file_size = 10;
                d.duration_ms = Some(5_000);
            }),
            with(2, "a.mp4", |d| {
                d.captured_at = Some(300);
                d.file_size = 30;
                d.duration_ms = Some(1_000);
            }),
            with(3, "a.mp4", |d| {
                d.captured_at = Some(200);
                d.file_size = 20;
                d.duration_ms = Some(9_000);
            }),
            with(4, "a.mp4", |d| d.file_size = 40), // captured_at 없음
        ],
    );
    let base = SearchRequest {
        text: "a".into(),
        ..Default::default()
    };

    // captured_at 구간 (닫힌 구간, 경계 포함) — 값이 없는 문서는 구간 필터에서 제외
    let mut r = base.clone();
    r.filters.captured_at = Range {
        min: Some(200),
        max: Some(300),
    };
    assert_eq!(ids(&idx, &r), [2, 3]);
    // 한쪽만 열린 구간
    r.filters.captured_at = Range {
        min: Some(250),
        max: None,
    };
    assert_eq!(ids(&idx, &r), [2]);
    // file_size · duration_ms
    let mut r = base.clone();
    r.filters.file_size = Range {
        min: Some(20),
        max: Some(30),
    };
    assert_eq!(ids(&idx, &r), [2, 3]);
    r.filters.duration_ms = Range {
        min: Some(2_000),
        max: None,
    };
    assert_eq!(ids(&idx, &r), [3]);
    // 빈 구간
    let mut r = base.clone();
    r.filters.captured_at = Range {
        min: Some(500),
        max: Some(400),
    };
    assert!(ids(&idx, &r).is_empty());

    // 촬영일 정렬 — 값 없는 문서는 양방향 모두 뒤
    let mut r = base.clone();
    r.sort = Sort::CapturedAtDesc;
    let order: Vec<u64> = idx.search(&r).unwrap().iter().map(|h| h.asset_id).collect();
    assert_eq!(order, [2, 3, 1, 4]);
    r.sort = Sort::CapturedAtAsc;
    let order: Vec<u64> = idx.search(&r).unwrap().iter().map(|h| h.asset_id).collect();
    assert_eq!(order, [1, 3, 2, 4]); // 값 없는 문서는 오름차순에서도 뒤
    r.sort = Sort::DurationMsDesc;
    let order: Vec<u64> = idx.search(&r).unwrap().iter().map(|h| h.asset_id).collect();
    assert_eq!(&order[..3], [3, 1, 2]);
}

#[test]
fn t_fts_11_pagination_and_empty_text_filter_only() {
    let dir = tempdir().unwrap();
    let docs: Vec<_> = (1..=7u64)
        .map(|i| with(i, "p.jpg", |d| d.captured_at = Some(i as i64)))
        .collect();
    let idx = indexed(&dir, &docs);
    let mut r = SearchRequest {
        sort: Sort::CapturedAtDesc,
        limit: 3,
        ..Default::default()
    };
    let page = |idx: &mam_search::SearchIndex, r: &SearchRequest| -> Vec<u64> {
        idx.search(r).unwrap().iter().map(|h| h.asset_id).collect()
    };
    assert_eq!(page(&idx, &r), [7, 6, 5]);
    r.offset = 3;
    assert_eq!(page(&idx, &r), [4, 3, 2]);
    r.offset = 6;
    assert_eq!(page(&idx, &r), [1]);
    r.offset = 100;
    assert!(page(&idx, &r).is_empty());
    r.limit = 0;
    assert!(page(&idx, &r).is_empty());
}

#[test]
fn t_fts_12_field_boost_ranks_filename_over_transcript() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "x.jpg", |d| d.transcript = "바다".into()),
            with(2, "y.jpg", |d| d.user_tags = vec!["바다".into()]),
            with(3, "바다.jpg", |_| {}),
            with(4, "z.jpg", |d| d.ai_description = "바다".into()),
        ],
    );
    let hits = idx.search(&req("바다", A)).unwrap();
    let order: Vec<u64> = hits.iter().map(|h| h.asset_id).collect();
    assert_eq!(order.len(), 4);
    // 파일명/태그(5) > 설명(3) > 전사(1)
    assert_eq!(order[3], 1);
    assert_eq!(order[2], 4);
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn t_fts_13_multi_word_and_unusual_queries_do_not_error() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "trip.jpg", |d| {
                d.user_tags = vec!["가족".into(), "바다".into()]
            }),
            with(2, "trip.jpg", |d| d.user_tags = vec!["가족".into()]),
        ],
    );
    // 단어 모두 일치 (AND)
    assert_eq!(ids(&idx, &req("가족 바다", A)), [1]);
    assert_eq!(ids(&idx, &req("trip 가족", A)), [1, 2]);
    // 문법 문자·기호만 있는 질의는 오류 없이 빈 결과
    for q in [
        "\"unterminated",
        "a:b",
        "(",
        "-",
        "*",
        "()",
        "    ",
        "!!!",
        "\u{0}",
    ] {
        let r = idx.search(&req(q, A));
        assert!(r.is_ok(), "query {q:?}");
    }
    // 기호·조사뿐인 단어는 AND 에서 빠진다 (I1 회귀)
    assert_eq!(ids(&idx, &req("!!!", A)), Vec::<u64>::new()); // 쓸 단어가 없으면 빈 결과
    assert_eq!(ids(&idx, &req("trip !!!", A)), [1, 2]);
    assert_eq!(ids(&idx, &req("가족 & 바다", A)), [1]);
    assert_eq!(ids(&idx, &req("가족 에서", A)), [1, 2]);
    // 빈 질의 + 필터 없음 = 전체
    assert_eq!(ids(&idx, &req("", A)), [1, 2]);
}

/// 분류: Happy (정렬·구간이 합성 필드를 읽는다) / Boundary (ms -> 초 내림) / Business rule (플래그 꺼짐 = 기존 동작, 같은 인덱스)
#[test]
fn t_capfb_fts_01_captured_fallback_flag_switches_sort_and_range_field_on_the_same_index() {
    // Arrange: 1,2 는 촬영일(100,300), 3 은 촬영일 없음 + 수정일 250.999초, 4 는 촬영일 없음 + 수정일 50초
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.jpg", |d| { d.captured_at = Some(100); d.modified_at = 9_000_000; }),
            with(2, "a.jpg", |d| { d.captured_at = Some(300); d.modified_at = 9_000_000; }),
            with(3, "a.jpg", |d| d.modified_at = 250_999),
            with(4, "a.jpg", |d| d.modified_at = 50_000),
        ],
    );
    let run = |fallback: bool, sort: Sort, min: Option<i64>, max: Option<i64>| {
        let mut r = SearchRequest { text: "a".into(), sort, ..Default::default() };
        r.filters.captured_fallback = fallback;
        r.filters.captured_at = Range { min, max };
        ids_sorted(&idx, &r)
    };

    // Act + Assert: 꺼짐 = 기존 captured_at (값 없는 문서는 뒤, 구간에서 제외)
    assert_eq!(run(false, Sort::CapturedAtDesc, None, None), [2, 1, 3, 4]);
    assert_eq!(run(false, Sort::CapturedAtDesc, Some(0), Some(1_000)), [2, 1]);
    // 켜짐 = captured_or_modified: 300 > 250(내림) > 100 > 50
    assert_eq!(run(true, Sort::CapturedAtDesc, None, None), [2, 3, 1, 4]);
    assert_eq!(run(true, Sort::CapturedAtAsc, None, None), [4, 1, 3, 2]);
    // 경계: 250.999초 -> 250 이므로 [250, 250] 구간에 걸리고 [251, ..] 에는 걸리지 않는다
    assert_eq!(run(true, Sort::CapturedAtDesc, Some(250), Some(250)), [3]);
    assert_eq!(run(true, Sort::CapturedAtDesc, Some(251), Some(299)), Vec::<u64>::new());
}

/// 정렬 순서를 보존해 asset_id 목록을 돌려준다.
fn ids_sorted(idx: &mam_search::SearchIndex, r: &SearchRequest) -> Vec<u64> {
    idx.search(r).unwrap().iter().map(|h| h.asset_id).collect()
}
