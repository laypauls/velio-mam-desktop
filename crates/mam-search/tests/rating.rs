//! 별점 `rating` 필드 — 범위 필터·정렬 (T-search-rating-*). 임시 디렉터리의 실제 Tantivy 인덱스.
//!
//! 분류: Happy (01, 03) / Boundary (01 경계·0 = 미평가·min > max, 02 값 클램프) /
//! Business rule (04 검색 단계 필터 — 상위 후보 뒤 후필터 아님, 05 정렬 동점) / Concurrency (06 커서 지문)

mod common;

use common::{doc, indexed};
use mam_search::coordinator::{
    CoordPage, CoordRequest, Direction, SearchHandle, SectionKind, SortField, SortSpec,
};
use mam_search::{AssetDoc, FilenameStrategy, Filters, Range, SearchError};
use tempfile::tempdir;

fn creq(text: &str) -> CoordRequest {
    CoordRequest {
        text: text.to_string(),
        strategy: FilenameStrategy::B,
        filters: Filters::default(),
        sort: SortSpec::default(),
        limit: 50,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

fn rated(id: u64, name: &str, rating: u8) -> AssetDoc {
    let mut d = doc(id, name);
    d.rating = rating;
    d
}

fn range(min: Option<u64>, max: Option<u64>) -> Filters {
    Filters {
        rating: Range { min, max },
        ..Default::default()
    }
}

fn page(h: &SearchHandle, r: &CoordRequest) -> CoordPage {
    h.search(r).unwrap()
}

/// 1..=6 번 자산: 별점 0,1,2,3,4,5
fn six() -> (tempfile::TempDir, mam_search::SearchIndex) {
    let dir = tempdir().unwrap();
    let docs: Vec<AssetDoc> = (0..=5u8)
        .map(|r| rated(u64::from(r) + 1, &format!("beach{r}.jpg"), r))
        .collect();
    let idx = indexed(&dir, &docs);
    (dir, idx)
}

#[test]
fn t_search_rating_01_range_exact_and_unrated_filters_include_their_bounds() {
    // Arrange
    let (_dir, idx) = six();
    let h = SearchHandle::for_index(&idx);
    let ids_of = |f: Filters| {
        let mut r = creq("");
        r.filters = f;
        let p = page(&h, &r);
        assert_eq!(p.total as usize, p.ids.len());
        p.ids
    };

    // Act + Assert
    assert_eq!(ids_of(Filters::default()), [1, 2, 3, 4, 5, 6], "no rating bound -> everything");
    assert_eq!(ids_of(range(Some(3), None)), [4, 5, 6], "min only (inclusive)");
    assert_eq!(ids_of(range(None, Some(2))), [1, 2, 3], "max only includes unrated");
    assert_eq!(ids_of(range(Some(2), Some(4))), [3, 4, 5], "both bounds inclusive");
    assert_eq!(ids_of(range(Some(4), Some(4))), [5], "min == max is an exact match");
    assert_eq!(ids_of(range(Some(0), Some(0))), [1], "0..0 = unrated only");
    assert_eq!(ids_of(range(Some(5), Some(5))), [6]);
    assert_eq!(ids_of(range(Some(4), Some(2))), Vec::<u64>::new(), "min > max matches nothing");
    assert_eq!(ids_of(range(Some(6), None)), Vec::<u64>::new(), "above the scale matches nothing");
}

#[test]
fn t_search_rating_02_out_of_scale_values_are_clamped_when_indexing() {
    // Arrange: 손상된 입력이 5 를 넘어도 색인은 0..=5 로 제한한다
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &[rated(1, "a.jpg", 200), rated(2, "b.jpg", 5)]);
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("");
    r.filters = range(Some(5), Some(5));

    // Act
    let p = page(&h, &r);

    // Assert
    assert_eq!(p.ids, [1, 2]);
}

#[test]
fn t_search_rating_03_keyword_and_other_filters_combine_with_the_rating_range() {
    // Arrange
    let dir = tempdir().unwrap();
    let mut fav = rated(3, "beach_fav.jpg", 5);
    fav.favorite = true;
    let idx = indexed(
        &dir,
        &[
            rated(1, "beach_a.jpg", 5),
            rated(2, "forest_b.jpg", 5),
            fav,
            rated(4, "beach_d.jpg", 1),
        ],
    );
    let h = SearchHandle::for_index(&idx);

    // Act
    let mut by_text = creq("beach");
    by_text.filters = range(Some(4), None);
    let mut by_fav = creq("");
    by_fav.filters = Filters {
        favorite_only: true,
        rating: Range { min: Some(5), max: None },
        ..Default::default()
    };

    // Assert
    let mut text_ids = page(&h, &by_text).ids;
    text_ids.sort_unstable();
    assert_eq!(text_ids, [1, 3], "keyword AND rating");
    assert_eq!(page(&h, &by_fav).ids, [3], "favorite AND rating");
}

#[test]
fn t_search_rating_04_the_filter_runs_inside_the_search_not_after_the_top_k() {
    // Arrange: 별점 5 가 3건뿐이고 나머지 200건이 모두 더 앞선 정렬 위치(asset_id 작음)에 있다
    let dir = tempdir().unwrap();
    let mut docs: Vec<AssetDoc> = (1..=200).map(|i| rated(i, &format!("n{i}.jpg"), 1)).collect();
    docs.extend((201..=203).map(|i| rated(i, &format!("n{i}.jpg"), 5)));
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("");
    r.filters = range(Some(5), Some(5));
    r.limit = 2;

    // Act: 2건씩 커서 페이징
    let first = page(&h, &r);
    r.cursor = first.next_cursor.clone();
    let second = page(&h, &r);

    // Assert: 후필터였다면 첫 페이지가 비었을 것이다
    assert_eq!((first.total, second.total), (3, 3));
    assert_eq!(first.ids, [201, 202]);
    assert_eq!(second.ids, [203]);
    assert!(second.next_cursor.is_none());
}

#[test]
fn t_search_rating_05_sort_by_rating_orders_by_value_and_ties_by_asset_id() {
    // Arrange: 별점 3,5,3,0,5
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            rated(1, "a.jpg", 3),
            rated(2, "b.jpg", 5),
            rated(3, "c.jpg", 3),
            rated(4, "d.jpg", 0),
            rated(5, "e.jpg", 5),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let order = |desc| {
        let mut r = creq("");
        r.sort = SortSpec { field: SortField::Rating, descending: desc };
        page(&h, &r).ids
    };

    // Act + Assert: 미평가(0)도 값이라 맨 뒤(내림)/맨 앞(오름) — 동점은 asset_id 오름차순
    assert_eq!(order(true), [2, 5, 1, 3, 4]);
    assert_eq!(order(false), [4, 1, 3, 2, 5]);

    // 커서 페이징도 같은 순서 (limit 2)
    let mut r = creq("");
    r.sort = SortSpec { field: SortField::Rating, descending: true };
    r.limit = 2;
    let mut seen = Vec::new();
    loop {
        let p = page(&h, &r);
        seen.extend(p.ids);
        match p.next_cursor {
            Some(c) => r.cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen, [2, 5, 1, 3, 4]);
}

#[test]
fn t_search_rating_06_changing_the_rating_filter_or_sort_makes_an_old_cursor_stale() {
    // Arrange
    let (_dir, idx) = six();
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("");
    r.filters = range(Some(1), None);
    r.limit = 2;
    let cursor = page(&h, &r).next_cursor.expect("more pages");

    // Act + Assert: 필터 변경
    let mut other = r.clone();
    other.filters = range(Some(2), None);
    other.cursor = Some(cursor.clone());
    assert!(matches!(h.search(&other), Err(SearchError::StaleCursor)));
    // 정렬 변경
    let mut sorted = r.clone();
    sorted.sort = SortSpec { field: SortField::Rating, descending: true };
    sorted.cursor = Some(cursor.clone());
    assert!(matches!(h.search(&sorted), Err(SearchError::StaleCursor)));
    // 같은 요청은 계속 유효
    r.cursor = Some(cursor);
    assert_eq!(page(&h, &r).ids, [4, 5]);
}

#[test]
fn t_search_rating_07_rating_sort_has_no_section_index_but_counts_the_filtered_total() {
    // Arrange
    let (_dir, idx) = six();
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("");
    r.sort = SortSpec { field: SortField::Rating, descending: true };
    r.filters = range(Some(2), None);

    // Act
    let s = h.section_index(&r, &|_| 0).unwrap();

    // Assert
    assert_eq!(s.kind, SectionKind::None);
    assert_eq!(s.total, 4);
}
