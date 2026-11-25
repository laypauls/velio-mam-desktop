//! 조정기 양방향 커서·위치 seek(`at`) 테스트 — 임시 디렉터리의 실제 Tantivy 인덱스.
//! 분류: T-page-01 Happy(왕복) / 02 Happy(위치) / 03 Happy+Boundary(seek) / 04 Boundary(범위 밖) /
//! 05 Validation / 06 Business rule(세대·지문) / 07 Business rule(필터·질의 + seek)

mod common;

use common::{doc, indexed};
use mam_search::coordinator::{
    CoordPage, CoordRequest, Direction, SearchHandle, SortField, SortSpec,
};
use mam_search::{AssetDoc, FilenameStrategy, Filters, MediaType, SearchError};
use tempfile::tempdir;

pub fn creq(sort: SortField, desc: bool, limit: usize) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters::default(),
        sort: SortSpec {
            field: sort,
            descending: desc,
        },
        limit,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

/// 촬영일 중복·값 없음(9의 배수), 크기 중복, 이름 다양, 수정일 중복.
fn dataset(n: u64) -> Vec<AssetDoc> {
    (1..=n)
        .map(|i| {
            let mut d = doc(i, &format!("n{}-photo{i}.jpg", i % 17));
            if i % 9 != 0 {
                d.captured_at = Some(1_700_000_000 + (i as i64 / 4) * 86_400 * 20);
            }
            d.file_size = (i * 7) % 50;
            d.modified_at = 1_700_000_000_000 + (i as i64 % 5) * 86_400_000;
            if i % 3 == 0 {
                d.media_type = MediaType::Video;
            }
            d
        })
        .collect()
}

fn forward_all(h: &SearchHandle, base: &CoordRequest) -> Vec<CoordPage> {
    let mut pages = Vec::new();
    let mut r = base.clone();
    loop {
        let p = h.search(&r).unwrap();
        let next = p.next_cursor.clone();
        pages.push(p);
        match next {
            Some(c) => r.cursor = Some(c),
            None => return pages,
        }
    }
}

fn flat(pages: &[CoordPage]) -> Vec<u64> {
    pages.iter().flat_map(|p| p.ids.clone()).collect()
}

const SORTS: [(SortField, bool); 9] = [
    (SortField::CapturedAt, true),
    (SortField::CapturedAt, false),
    (SortField::ModifiedAt, true),
    (SortField::Name, false),
    (SortField::Name, true),
    (SortField::Size, true),
    (SortField::Size, false),
    (SortField::Duration, false),
    (SortField::Relevance, true),
];

#[test]
fn t_page_01_backward_pages_mirror_forward_pages_for_every_sort() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    for (field, desc) in SORTS {
        let base = creq(field, desc, 20);
        let forward = forward_all(&h, &base);
        let all = flat(&forward);
        assert_eq!(all.len(), 137, "{field:?} {desc}: every document once");

        // Act: 마지막 페이지에서 prevCursor 만 따라 거슬러 올라간다
        let mut back: Vec<CoordPage> = Vec::new();
        let mut cur = forward.last().unwrap().clone();
        while let Some(prev) = cur.prev_cursor.clone() {
            let mut r = base.clone();
            r.cursor = Some(prev);
            r.direction = Direction::Backward;
            cur = h.search(&r).unwrap();
            back.push(cur.clone());
        }

        // Assert: 거슬러 올라온 페이지 = 순방향 페이지(뒤에서 두 번째부터)와 같은 항목·같은 위치
        assert_eq!(back.len(), forward.len() - 1, "{field:?} {desc}");
        for (b, f) in back.iter().zip(forward.iter().rev().skip(1)) {
            assert_eq!(b.ids, f.ids, "{field:?} {desc}: display order is preserved");
            assert_eq!(b.start_offset, f.start_offset, "{field:?} {desc}");
            assert_eq!(b.next_cursor, f.next_cursor, "{field:?} {desc}");
            assert_eq!(b.total, 137);
        }
        assert_eq!(back.last().unwrap().start_offset, 0);
        assert!(
            back.last().unwrap().prev_cursor.is_none(),
            "start has no prev"
        );
        assert!(forward[0].prev_cursor.is_none());
    }
}

#[test]
fn t_page_01b_backward_with_unaligned_cursor_returns_the_items_just_before_it() {
    // Arrange: 커서 = 순방향 페이지 경계가 아닌 임의 항목(7번째)의 키
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(60));
    let h = SearchHandle::for_index(&idx);
    let base = creq(SortField::CapturedAt, true, 10);
    let all = flat(&forward_all(&h, &creq(SortField::CapturedAt, true, 200)));
    // 항목 k 의 커서 = "k 를 첫 항목으로 하는 페이지"의 prevCursor 가 아니라, at=k 페이지의 prevCursor (첫 항목의 키)
    let mut at = base.clone();
    at.at = Some(25);
    let anchor = h.search(&at).unwrap();
    assert_eq!(anchor.ids, all[25..35]);

    // Act
    let mut r = base.clone();
    r.cursor = anchor.prev_cursor.clone();
    r.direction = Direction::Backward;
    let before = h.search(&r).unwrap();

    // Assert: 25 번째 항목 바로 앞 10건
    assert_eq!(before.ids, all[15..25]);
    assert_eq!(before.start_offset, 15);
    // 첫 페이지 앞쪽 경계: 5건만 남은 곳에서는 5건만 돌려주고 prev 가 없다
    let mut at5 = base.clone();
    at5.at = Some(5);
    let mut r2 = base.clone();
    r2.cursor = h.search(&at5).unwrap().prev_cursor;
    r2.direction = Direction::Backward;
    let head = h.search(&r2).unwrap();
    assert_eq!(head.ids, all[0..5]);
    assert_eq!(head.start_offset, 0);
    assert!(head.prev_cursor.is_none());
}

#[test]
fn t_page_02_forward_cursor_pages_report_their_global_position() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    for (field, desc) in SORTS {
        // Act
        let pages = forward_all(&h, &creq(field, desc, 25));
        // Assert
        let mut expect = 0;
        for p in &pages {
            assert_eq!(p.start_offset, expect, "{field:?} {desc}");
            expect += p.ids.len() as u64;
        }
        assert_eq!(expect, 137);
        let g = pages[0].generation;
        assert!(pages.iter().all(|p| p.generation == g), "one generation");
    }
}

#[test]
fn t_page_03_seek_equals_the_same_slice_of_sequential_paging() {
    // Arrange: total 137 -> 앞 절반(정방향 수집)과 뒤 절반(끝에서 수집) 경로를 모두 지난다
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    for (field, desc) in SORTS {
        let limit = 20;
        let all = flat(&forward_all(&h, &creq(field, desc, 200)));
        let total = all.len();
        // 시작·1·경계(limit±1)·중간(68/69)·마지막 페이지·마지막 항목, 값 없음 구간(촬영일 정렬: 맨 뒤 15건)
        let positions = [
            0,
            1,
            limit - 1,
            limit,
            limit + 1,
            67,
            68,
            69,
            70,
            100,
            total - limit,
            total - limit + 1,
            total - 3,
            total - 1,
        ];
        for at in positions {
            // Act
            let mut r = creq(field, desc, limit);
            r.at = Some(at);
            let page = h.search(&r).unwrap();

            // Assert
            let end = (at + limit).min(total);
            assert_eq!(page.ids, all[at..end], "{field:?} {desc} at={at}");
            assert_eq!(page.start_offset, at as u64);
            assert_eq!(page.total, total as u64);
            assert_eq!(
                page.next_cursor.is_some(),
                end < total,
                "{field:?} {desc} at={at}"
            );
            assert_eq!(page.prev_cursor.is_some(), at > 0);

            // 이어서: next/prev 커서는 seek 페이지에서도 정확히 이어진다
            if let Some(c) = page.next_cursor.clone() {
                let mut n = creq(field, desc, limit);
                n.cursor = Some(c);
                let next = h.search(&n).unwrap();
                assert_eq!(
                    next.ids,
                    all[end..(end + limit).min(total)],
                    "{field:?} {desc} next of at={at}"
                );
                assert_eq!(next.start_offset, end as u64);
            }
            if let Some(c) = page.prev_cursor.clone() {
                let mut p = creq(field, desc, limit);
                p.cursor = Some(c);
                p.direction = Direction::Backward;
                let prev = h.search(&p).unwrap();
                let from = at.saturating_sub(limit);
                assert_eq!(prev.ids, all[from..at], "{field:?} {desc} prev of at={at}");
                assert_eq!(prev.start_offset, from as u64);
            }
        }
    }
}

#[test]
fn t_page_03b_seek_into_the_missing_value_region_keeps_nulls_last() {
    // Arrange: 촬영일 없는 문서(9의 배수 15건)는 오름·내림 모두 맨 뒤
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    for desc in [true, false] {
        let mut r = creq(SortField::CapturedAt, desc, 10);
        r.at = Some(137 - 15);
        // Act
        let page = h.search(&r).unwrap();
        // Assert: 이 페이지는 전부 값 없는 문서, asset_id 오름차순
        assert_eq!(page.ids.len(), 10);
        assert!(
            page.ids.iter().all(|id| id % 9 == 0),
            "desc={desc}: {:?}",
            page.ids
        );
        assert!(page.ids.windows(2).all(|w| w[0] < w[1]));
    }
}

#[test]
fn t_page_04_seek_past_the_end_returns_an_empty_page_at_total() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(30));
    let h = SearchHandle::for_index(&idx);
    for at in [30usize, 31, 10_000] {
        // Act
        let mut r = creq(SortField::Name, false, 10);
        r.at = Some(at);
        let page = h.search(&r).unwrap();
        // Assert
        assert!(page.ids.is_empty(), "at={at}");
        assert_eq!((page.total, page.start_offset), (30, 30));
        assert!(page.next_cursor.is_none() && page.prev_cursor.is_none());
    }
    // 결과 없음 + at 0 도 빈 페이지 (오류가 아니다)
    let mut none = creq(SortField::Name, false, 10);
    none.filters.media_types = vec![MediaType::Audio];
    none.at = Some(0);
    let page = h.search(&none).unwrap();
    assert_eq!((page.ids.len(), page.total, page.start_offset), (0, 0, 0));
}

#[test]
fn t_page_05_conflicting_or_incomplete_paging_arguments_are_invalid_input() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(30));
    let h = SearchHandle::for_index(&idx);
    let cursor = h
        .search(&creq(SortField::Name, false, 10))
        .unwrap()
        .next_cursor
        .unwrap();

    // cursor + at
    let mut both = creq(SortField::Name, false, 10);
    both.cursor = Some(cursor.clone());
    both.at = Some(3);
    // backward 인데 커서 없음 / backward + at
    let mut no_cursor = creq(SortField::Name, false, 10);
    no_cursor.direction = Direction::Backward;
    let mut back_at = no_cursor.clone();
    back_at.at = Some(2);
    // 잘못된 커서 + backward
    let mut bad = creq(SortField::Name, false, 10);
    bad.direction = Direction::Backward;
    bad.cursor = Some("garbage".into());

    // Act + Assert
    for (name, r) in [
        ("cursor+at", both),
        ("backward no cursor", no_cursor),
        ("backward+at", back_at),
        ("bad cursor", bad),
    ] {
        assert!(
            matches!(h.search(&r), Err(SearchError::InvalidInput(_))),
            "{name}"
        );
    }
    // limit 검사는 seek·backward 에도 적용된다
    let mut zero = creq(SortField::Name, false, 0);
    zero.at = Some(0);
    assert!(matches!(h.search(&zero), Err(SearchError::InvalidInput(_))));
}

#[test]
fn t_page_06_cursors_from_another_generation_or_sort_are_stale_in_both_directions() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(60));
    let h = SearchHandle::for_index(&idx);
    let mut at = creq(SortField::Name, false, 10);
    at.at = Some(20);
    let page = h.search(&at).unwrap();
    let prev = page.prev_cursor.unwrap();
    let next = page.next_cursor.unwrap();
    let other_gen = SearchHandle::for_index(&idx);

    for (dir_, cursor) in [
        (Direction::Backward, prev.clone()),
        (Direction::Forward, next.clone()),
    ] {
        // 다른 세대
        let mut r = creq(SortField::Name, false, 10);
        r.cursor = Some(cursor.clone());
        r.direction = dir_;
        assert!(
            matches!(other_gen.search(&r), Err(SearchError::StaleCursor)),
            "generation {dir_:?}"
        );
        // 다른 정렬
        r.sort.field = SortField::Size;
        assert!(
            matches!(h.search(&r), Err(SearchError::StaleCursor)),
            "sort {dir_:?}"
        );
    }
    assert_ne!(page.generation, other_gen.generation());
}

#[test]
fn t_page_07_seek_and_backward_respect_filters_and_text_queries() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    let mut base = creq(SortField::CapturedAt, true, 7);
    base.filters.media_types = vec![MediaType::Video];
    base.text = "photo".into();
    let all = flat(&forward_all(&h, &{
        let mut b = base.clone();
        b.limit = 200;
        b
    }));
    assert_eq!(all.len(), 137 / 3);
    assert!(all.iter().all(|id| id % 3 == 0));

    // Act + Assert: 필터·질의가 있어도 seek = 순차의 같은 구간
    for at in [0, 6, 7, 20, 44, all.len() - 1] {
        let mut r = base.clone();
        r.at = Some(at);
        let p = h.search(&r).unwrap();
        assert_eq!(p.ids, all[at..(at + 7).min(all.len())], "at={at}");
        assert_eq!(p.total, all.len() as u64);
    }
}

#[test]
fn t_page_08_a_deleted_cursor_item_still_yields_consistent_positions() {
    // Arrange: 커서가 가리키던 문서가 지워져도 키 비교만으로 이어진다
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(50));
    let h = SearchHandle::for_index(&idx);
    let first = h.search(&creq(SortField::Size, false, 10)).unwrap();
    let cursor = first.next_cursor.clone().unwrap();
    let last_id = *first.ids.last().unwrap();
    idx.delete(last_id).unwrap();
    idx.commit().unwrap();

    // Act
    let mut r = creq(SortField::Size, false, 10);
    r.cursor = Some(cursor);
    let next = h.search(&r).unwrap();

    // Assert: 지워진 항목은 세지 않으므로 한 건 줄어든 목록에서 앞 9건 뒤에서 시작한다
    assert_eq!(next.total, 49);
    assert_eq!(next.start_offset, 9);
    assert_eq!(next.ids.len(), 10);
}

#[test]
fn t_page_09_relevance_paging_with_a_text_query_round_trips_and_seeks() {
    // Arrange: BM25 점수 정렬 (문서가 바뀌지 않는 동안 안정)
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset(137));
    let h = SearchHandle::for_index(&idx);
    let mut base = creq(SortField::Relevance, true, 9);
    base.text = "photo".into();
    let forward = forward_all(&h, &base);
    let all = flat(&forward);
    assert!(all.len() > 9, "needs several pages: {}", all.len());

    // Act + Assert: 뒤로 한 페이지, 그리고 seek
    let last = forward.last().unwrap();
    let mut b = base.clone();
    b.cursor = last.prev_cursor.clone();
    b.direction = Direction::Backward;
    assert_eq!(h.search(&b).unwrap().ids, forward[forward.len() - 2].ids);
    for at in [0, 4, all.len() / 2, all.len() - 1] {
        let mut s = base.clone();
        s.at = Some(at);
        assert_eq!(
            h.search(&s).unwrap().ids,
            all[at..(at + 9).min(all.len())],
            "at={at}"
        );
    }
}
