//! 숨김 폴더 제외 (`Filters::hidden_folder_ids`) — 임시 디렉터리의 실제 Tantivy 인덱스.
//! 분류: Business rule (01 제외·다른 폴더 유지, 04 폴더 범위 결합, 05 후보 집합) / Boundary (02 빈 목록, 03 텍스트 없는 질의,
//! 06 없는 id)

mod common;

use common::{doc, indexed};
use mam_search::coordinator::{CoordRequest, Direction, SearchHandle, SortSpec};
use mam_search::search::FolderFilter;
use mam_search::{AssetDoc, FilenameStrategy, Filters};
use tempfile::tempdir;

fn creq(text: &str, filters: Filters) -> CoordRequest {
    CoordRequest {
        text: text.to_string(),
        strategy: FilenameStrategy::B,
        filters,
        sort: SortSpec::default(),
        limit: 50,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

fn in_folder(id: u64, name: &str, folder: Option<u64>, dir_key: &str) -> AssetDoc {
    let mut d = doc(id, name);
    d.folder_id = folder;
    d.dir_key = dir_key.into();
    d
}

fn hidden(ids: &[u64]) -> Filters {
    Filters {
        hidden_folder_ids: ids.to_vec(),
        ..Default::default()
    }
}

/// 폴더 7 (루트 + a/b), 폴더 8, 폴더 밖(None) 에 모두 "common" 이 들어 있다.
fn docs() -> Vec<AssetDoc> {
    vec![
        in_folder(1, "common one.jpg", Some(7), ""),
        in_folder(2, "common two.jpg", Some(7), "a/b"),
        in_folder(3, "common three.jpg", Some(8), ""),
        in_folder(4, "common four.jpg", None, ""),
    ]
}

fn ids(h: &SearchHandle, r: &CoordRequest) -> Vec<u64> {
    let mut v = h.search(r).unwrap().ids;
    v.sort_unstable();
    v
}

#[test]
fn t_hide_search_01_hidden_folder_is_excluded_including_nested_dirs_and_other_folders_are_kept() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);

    // Act + Assert
    assert_eq!(ids(&h, &creq("common", hidden(&[7]))), [3, 4], "폴더 7 의 하위(a/b) 포함 제외");
    assert_eq!(ids(&h, &creq("common", hidden(&[8]))), [1, 2, 4]);
    assert_eq!(ids(&h, &creq("common", hidden(&[7, 8]))), [4], "여러 폴더");
    assert_eq!(h.search(&creq("common", hidden(&[7]))).unwrap().total, 2);
}

#[test]
fn t_hide_search_02_empty_list_leaves_results_unchanged() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);

    // Act + Assert
    assert_eq!(
        ids(&h, &creq("common", hidden(&[]))),
        ids(&h, &creq("common", Filters::default()))
    );
    assert_eq!(ids(&h, &creq("common", Filters::default())), [1, 2, 3, 4]);
}

#[test]
fn t_hide_search_03_text_less_query_is_pure_must_not_and_still_works() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);

    // Act + Assert
    assert_eq!(ids(&h, &creq("", hidden(&[7]))), [3, 4]);
    assert_eq!(ids(&h, &creq("", hidden(&[7, 8]))), [4]);
}

#[test]
fn t_hide_search_04_combined_with_a_folder_filter_the_exclusion_still_applies_when_given() {
    // Arrange: 호출자(검색 서비스)는 폴더 범위가 있으면 목록을 비우지만, 검색 계층 자체는 주어진 절을 모두 적용한다
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);
    let scope = |id| FolderFilter {
        folder_id: id,
        dir_key: String::new(),
        recursive: true,
    };

    // Act + Assert
    let f = |hidden_ids: &[u64], folder: u64| Filters {
        folder: Some(scope(folder)),
        hidden_folder_ids: hidden_ids.to_vec(),
        ..Default::default()
    };
    assert_eq!(ids(&h, &creq("common", f(&[7], 7))), Vec::<u64>::new(), "같은 폴더 범위 + 제외 = 없음");
    assert_eq!(ids(&h, &creq("common", f(&[7], 8))), [3], "다른 폴더 범위는 영향 없음");
    assert_eq!(ids(&h, &creq("common", f(&[], 7))), [1, 2], "목록 비움 = 범위 내용 표시");
}

#[test]
fn t_hide_search_05_candidate_sets_for_the_vector_path_honor_the_exclusion() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);
    let sorted = |mut v: Vec<u64>| {
        v.sort_unstable();
        v
    };

    // Act
    let filtered = sorted(h.filtered_asset_ids(&hidden(&[7])).unwrap());
    let (_, stamped) = h.filtered_asset_ids_stamped(&hidden(&[7])).unwrap();
    let docset = sorted(h.filtered_asset_ids_docset(&hidden(&[7])).unwrap());

    // Assert
    assert_eq!(filtered, [3, 4]);
    assert_eq!(sorted(stamped), [3, 4]);
    assert_eq!(docset, [3, 4]);
}

#[test]
fn t_hide_search_06_unknown_folder_id_excludes_nothing() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs());
    let h = SearchHandle::for_index(&idx);

    // Act + Assert
    assert_eq!(ids(&h, &creq("common", hidden(&[999]))), [1, 2, 3, 4]);
}
