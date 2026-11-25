//! 컬렉션 순서 정렬(`SortField::CollectionOrder`) 조정기 테스트 (T-coord-co-*) — 임시 디렉터리의 실제 Tantivy 인덱스.
//!
//! 분류: Happy (01 02 04 05 06) / Boundary (03 맵에 없는 문서·02 NULL 등급, 06 seek 양쪽 절반, 10 부호·극값) /
//! Business rule (07 질의·필터 동시 적용, 08 구간 없음) / Validation (09 맵 없음)

mod common;

use std::sync::Arc;

use common::{doc, indexed};
use mam_search::coordinator::{
    CollectionOrder, CoordPage, CoordRequest, Direction, SearchHandle, SectionKind, SortField,
    SortSpec,
};
use mam_search::{AssetDoc, FilenameStrategy, Filters, MediaType, SearchError, SearchIndex};
use tempfile::{TempDir, tempdir};

const COLLECTION: u64 = 1;

/// 1..=40 위치 있음(섞인 순서·음수 포함), 41..=50 위치 NULL, 51..=60 은 컬렉션 소속이지만 맵에 없음(색인 지연), 61..=65 는 비소속.
fn dataset() -> Vec<AssetDoc> {
    (1..=65)
        .map(|i| {
            let name = if i % 2 == 0 {
                format!("even-clip{i}.mp4")
            } else {
                format!("odd-photo{i}.jpg")
            };
            let mut d = doc(i, &name);
            if i <= 60 {
                d.collection_ids = vec![COLLECTION];
            }
            if i % 2 == 0 {
                d.media_type = MediaType::Video;
            }
            d
        })
        .collect()
}

fn position_of(i: i64) -> i64 {
    ((i * 37) % 40 - 20) * 1000 // 1..=40 에 대해 서로 다른 값, 음수 포함
}

fn added_at_of(i: i64) -> i64 {
    (i * 13) % 10 // 41..=50 에 대해 서로 다른 값
}

fn order_map() -> Arc<CollectionOrder> {
    let rows = (1..=50i64).map(|i| {
        if i <= 40 {
            (i, Some(position_of(i)), 5_000 + i)
        } else {
            (i, None, added_at_of(i))
        }
    });
    Arc::new(CollectionOrder::from_rows(rows))
}

/// 기대 순서: 위치 있는 행(위치순) -> NULL 행(added_at 순) -> 맵에 없는 문서(asset_id 순, 방향 무관).
fn expected(desc: bool) -> Vec<u64> {
    let mut positioned: Vec<i64> = (1..=40).collect();
    positioned.sort_by_key(|i| position_of(*i));
    let mut nulls: Vec<i64> = (41..=50).collect();
    nulls.sort_by_key(|i| (added_at_of(*i), *i));
    if desc {
        positioned.reverse();
        nulls.reverse();
    }
    let mut out: Vec<u64> = positioned
        .into_iter()
        .chain(nulls)
        .map(|i| i as u64)
        .collect();
    out.extend(51..=60u64);
    out
}

fn creq(desc: bool, limit: usize) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters {
            collection_id: Some(COLLECTION),
            ..Filters::default()
        },
        sort: SortSpec {
            field: SortField::CollectionOrder,
            descending: desc,
        },
        limit,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: Some(order_map()),
    }
}

/// 인덱스와 핸들 (`SearchIndex` 가 살아 있어야 핸들이 검색할 수 있다).
fn setup() -> (TempDir, SearchIndex, SearchHandle) {
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &dataset());
    let h = SearchHandle::for_index(&idx);
    (dir, idx, h)
}

fn walk(h: &SearchHandle, base: &CoordRequest) -> Vec<CoordPage> {
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

#[test]
fn t_coord_co_01_ascending_and_descending_follow_the_collection_positions() {
    let (_dir, _idx, h) = setup();

    let asc = h.search(&creq(false, 200)).unwrap();
    let desc = h.search(&creq(true, 200)).unwrap();

    assert_eq!(asc.ids, expected(false));
    assert_eq!(desc.ids, expected(true));
    assert_eq!((asc.total, asc.start_offset), (60, 0)); // 비소속 61..=65 는 필터가 뺀다
}

#[test]
fn t_coord_co_02_rows_without_a_position_sort_after_positioned_rows_in_both_directions() {
    let (_dir, _idx, h) = setup();

    for desc in [false, true] {
        let ids = h.search(&creq(desc, 200)).unwrap().ids;
        let last_positioned = ids.iter().rposition(|i| *i <= 40).unwrap();
        let first_null = ids.iter().position(|i| (41..=50).contains(i)).unwrap();
        assert!(
            last_positioned < first_null,
            "desc={desc}: NULL 등급은 항상 뒤"
        );
    }
}

#[test]
fn t_coord_co_03_documents_missing_from_the_map_are_last_in_both_directions() {
    let (_dir, _idx, h) = setup();

    for desc in [false, true] {
        let ids = h.search(&creq(desc, 200)).unwrap().ids;
        assert_eq!(&ids[50..], (51..=60).collect::<Vec<u64>>(), "desc={desc}");
    }
}

#[test]
fn t_coord_co_04_cursor_walk_visits_every_document_exactly_once_in_order() {
    let (_dir, _idx, h) = setup();

    for desc in [false, true] {
        let pages = walk(&h, &creq(desc, 7));
        assert_eq!(flat(&pages), expected(desc), "desc={desc}");
        assert_eq!(pages.len(), 9); // 60 / 7 -> 9쪽
        assert!(
            pages
                .iter()
                .enumerate()
                .all(|(i, p)| p.start_offset == (i * 7) as u64)
        );
    }
}

#[test]
fn t_coord_co_05_backward_paging_returns_the_same_pages_in_display_order() {
    let (_dir, _idx, h) = setup();
    let base = creq(false, 8);
    let forward = walk(&h, &base);

    let mut back: Vec<CoordPage> = Vec::new();
    let mut cur = forward.last().unwrap().clone();
    while let Some(prev) = cur.prev_cursor.clone() {
        let mut r = base.clone();
        r.cursor = Some(prev);
        r.direction = Direction::Backward;
        cur = h.search(&r).unwrap();
        back.push(cur.clone());
    }

    assert_eq!(back.len(), forward.len() - 1);
    for (b, f) in back.iter().zip(forward.iter().rev().skip(1)) {
        assert_eq!((&b.ids, b.start_offset), (&f.ids, f.start_offset));
    }
}

#[test]
fn t_coord_co_06_seek_works_in_both_halves_of_the_result() {
    let (_dir, _idx, h) = setup();
    let want = expected(false);

    for at in [0usize, 5, 30, 31, 45, 59] {
        let mut r = creq(false, 10);
        r.at = Some(at);
        let p = h.search(&r).unwrap();
        assert_eq!(p.ids, want[at..(at + 10).min(60)], "at={at}");
        assert_eq!(p.start_offset, at as u64);
    }
}

#[test]
fn t_coord_co_07_text_and_filters_narrow_the_set_but_keep_the_collection_order() {
    let (_dir, _idx, h) = setup();

    // 질의: 짝수 파일명만
    let mut by_text = creq(false, 200);
    by_text.text = "even".into();
    let text_ids = h.search(&by_text).unwrap().ids;
    // 필터: 동영상(짝수)만 — 같은 집합
    let mut by_filter = creq(false, 200);
    by_filter.filters.media_types = vec![MediaType::Video];
    let filter_ids = h.search(&by_filter).unwrap().ids;

    let want: Vec<u64> = expected(false)
        .into_iter()
        .filter(|i| i % 2 == 0)
        .collect();
    assert_eq!(text_ids, want);
    assert_eq!(filter_ids, want);
}

#[test]
fn t_coord_co_08_section_index_has_no_sections_for_collection_order() {
    let (_dir, _idx, h) = setup();

    let s = h.section_index(&creq(false, 1), &|_| 0).unwrap();

    assert_eq!(s.kind, SectionKind::None);
    assert_eq!(s.total, 60);
}

#[test]
fn t_coord_co_09_missing_map_is_invalid_input() {
    let (_dir, _idx, h) = setup();
    let mut r = creq(false, 10);
    r.collection_order = None;

    let err = h.search(&r).unwrap_err();

    assert!(matches!(err, SearchError::InvalidInput(_)), "{err:?}");
}

#[test]
fn t_coord_co_10_map_rows_use_signed_positions_and_ignore_invalid_ids() {
    // 음수·i64 극값 위치도 부호 있는 순서로 정렬되고, asset_id <= 0 행은 무시된다
    let dir = tempdir().unwrap();
    let docs: Vec<AssetDoc> = (1..=4).map(|i| doc(i, &format!("f{i}.jpg"))).collect();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);
    let map = CollectionOrder::from_rows([
        (1, Some(i64::MAX), 0),
        (2, Some(i64::MIN), 0),
        (3, Some(-1), 0),
        (4, Some(0), 0),
        (-5, Some(0), 0),
        (0, None, 0),
    ]);
    assert_eq!(map.len(), 4);
    let mut r = creq(false, 10);
    r.filters = Filters::default();
    r.collection_order = Some(Arc::new(map));

    assert_eq!(h.search(&r).unwrap().ids, [2, 3, 4, 1]);
}
