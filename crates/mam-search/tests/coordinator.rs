//! 검색 조정기 테스트 — 임시 디렉터리의 실제 Tantivy 인덱스.
//! 분류: T-coord-01 Happy / 02 Business rule / 03 Business rule / 04 Happy / 05 Concurrency /
//! 06 Validation / 07 Boundary / 08 Business rule

mod common;

use common::{doc, indexed};
use mam_search::coordinator::{CoordPage, CoordRequest, Direction, SearchHandle, SortField, SortSpec};
use mam_search::search::FolderFilter;
use mam_search::{AssetDoc, FilenameStrategy, Filters, MediaType, Presence, Range, SearchError};
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

fn sorted_by(field: SortField, descending: bool) -> SortSpec {
    SortSpec { field, descending }
}

fn with(id: u64, name: &str, f: impl FnOnce(&mut AssetDoc)) -> AssetDoc {
    let mut d = doc(id, name);
    f(&mut d);
    d
}

fn search(h: &SearchHandle, r: &CoordRequest) -> CoordPage {
    h.search(r).unwrap()
}

#[test]
fn t_coord_01_keyword_with_media_type_filter_is_ranked_by_relevance() {
    // Arrange: 파일명 일치(가중치 높음) vs 메모 일치 vs 다른 유형
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "notes.mp4", |d| {
                d.media_type = MediaType::Video;
                d.user_memo = "beach trip".into();
            }),
            with(2, "beach.mp4", |d| d.media_type = MediaType::Video),
            with(3, "beach.jpg", |_| {}),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("beach");
    r.filters.media_types = vec![MediaType::Video];

    // Act
    let page = search(&h, &r);

    // Assert
    assert_eq!(
        page.ids,
        [2, 1],
        "file-name match outranks memo match; image excluded"
    );
    assert_eq!(page.total, 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn t_coord_02_scope_favorite_collection_and_tag_filters_apply_inside_the_search() {
    // Arrange: 폴더 7 (a, a/b), 폴더 8 (루트). "common" 이 100건에 일치하지만 필터 대상은 관련도 꼴찌 3건
    let dir = tempdir().unwrap();
    let mut docs: Vec<AssetDoc> = (1..=100).map(|i| with(i, "common.jpg", |_| {})).collect();
    docs.push(with(101, "x.jpg", |d| {
        d.user_memo = "common".into();
        d.favorite = true;
        d.folder_id = Some(7);
        d.dir_key = "a/b".into();
        d.collection_ids = vec![3];
        d.tag_ids = vec![1, 2];
    }));
    docs.push(with(102, "y.jpg", |d| {
        d.user_memo = "common".into();
        d.folder_id = Some(7);
        d.dir_key = "a".into();
        d.tag_ids = vec![1];
    }));
    docs.push(with(103, "z.jpg", |d| {
        d.user_memo = "common".into();
        d.folder_id = Some(8);
        d.dir_key = String::new();
    }));
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);
    let ids_of = |f: Filters| {
        let mut r = creq("common");
        r.limit = 5; // 후필터였다면 상위 5건(파일명 일치)에서 모두 탈락한다
        r.filters = f;
        let mut v = search(&h, &r).ids;
        v.sort_unstable();
        v
    };
    let folder = |id, dir: &str, recursive| Filters {
        folder: Some(FolderFilter {
            folder_id: id,
            dir_key: dir.into(),
            recursive,
        }),
        ..Default::default()
    };

    // Act + Assert
    assert_eq!(ids_of(folder(7, "", true)), [101, 102], "root recursive");
    assert_eq!(
        ids_of(folder(7, "", false)),
        Vec::<u64>::new(),
        "nothing directly in root of 7"
    );
    assert_eq!(
        ids_of(folder(7, "a", true)),
        [101, 102],
        "a recursive includes a/b"
    );
    assert_eq!(
        ids_of(folder(7, "a", false)),
        [102],
        "a non-recursive excludes a/b"
    );
    assert_eq!(ids_of(folder(7, "a/b", false)), [101]);
    assert_eq!(ids_of(folder(8, "", false)), [103]);
    assert_eq!(
        ids_of(folder(9, "", true)),
        Vec::<u64>::new(),
        "unknown folder"
    );
    assert_eq!(
        ids_of(Filters {
            favorite_only: true,
            ..Default::default()
        }),
        [101]
    );
    assert_eq!(
        ids_of(Filters {
            collection_id: Some(3),
            ..Default::default()
        }),
        [101]
    );
    assert_eq!(
        ids_of(Filters {
            tag_ids: vec![1],
            ..Default::default()
        }),
        [101, 102]
    );
    assert_eq!(
        ids_of(Filters {
            tag_ids: vec![1, 2],
            ..Default::default()
        }),
        [101],
        "tags are ANDed"
    );
}

#[test]
fn t_coord_03_offline_and_missing_are_included_or_excluded_by_filter() {
    // Arrange: 볼륨 1 온라인, 볼륨 2 오프라인, 자산 4 는 파일 없음
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.jpg", |_| {}),
            with(2, "b.jpg", |d| d.volume_id = 2),
            with(3, "c.jpg", |_| {}),
            with(4, "d.jpg", |d| d.presence = Presence::Missing),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let ids_of = |f: Filters| {
        let mut r = creq("");
        r.filters = f;
        let mut v = search(&h, &r).ids;
        v.sort_unstable();
        v
    };

    // Act + Assert (PL5 기본: 오프라인 표시, 파일 없음 숨김)
    assert_eq!(
        ids_of(Filters {
            presence: Some(Presence::Present),
            ..Default::default()
        }),
        [1, 2, 3]
    );
    assert_eq!(
        ids_of(Filters::default()),
        [1, 2, 3, 4],
        "include everything"
    );
    assert_eq!(
        ids_of(Filters {
            online_volume_ids: Some(vec![1]),
            presence: Some(Presence::Present),
            ..Default::default()
        }),
        [1, 3],
        "offline volume excluded"
    );
    assert!(
        ids_of(Filters {
            online_volume_ids: Some(vec![]),
            ..Default::default()
        })
        .is_empty(),
        "no online volume -> nothing"
    );
}

#[test]
fn t_coord_04_sort_fields_asc_desc_with_ties_by_asset_id_and_missing_last() {
    // Arrange: 값 중복(동점)과 값 없음 포함
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "Banana.jpg", |d| {
                d.captured_at = Some(200);
                d.file_size = 50;
                d.duration_ms = Some(5);
                d.modified_at = 30;
            }),
            with(2, "apple.jpg", |d| {
                d.captured_at = Some(100);
                d.file_size = 50;
                d.duration_ms = Some(9);
                d.modified_at = 10;
            }),
            with(3, "cherry.jpg", |d| {
                d.captured_at = Some(100);
                d.file_size = 70;
                d.modified_at = 20;
            }),
            with(4, "가나.jpg", |d| d.file_size = 10),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let order = |f, desc| {
        let mut r = creq("");
        r.sort = sorted_by(f, desc);
        search(&h, &r).ids
    };

    // Act + Assert
    assert_eq!(
        order(SortField::CapturedAt, true),
        [1, 2, 3, 4],
        "ties by asset_id asc, missing last"
    );
    assert_eq!(
        order(SortField::CapturedAt, false),
        [2, 3, 1, 4],
        "missing last in asc too"
    );
    assert_eq!(
        order(SortField::ModifiedAt, true),
        [1, 3, 2, 4],
        "asset 4 has modified_at 0"
    );
    assert_eq!(order(SortField::Size, true), [3, 1, 2, 4]);
    assert_eq!(order(SortField::Size, false), [4, 1, 2, 3]);
    assert_eq!(order(SortField::Duration, true), [2, 1, 3, 4]);
    assert_eq!(
        order(SortField::Name, false),
        [2, 1, 3, 4],
        "case-insensitive, Hangul after Latin"
    );
    assert_eq!(order(SortField::Name, true), [4, 3, 1, 2]);
    assert_eq!(
        order(SortField::Relevance, true),
        [1, 2, 3, 4],
        "no text -> asset_id order"
    );
}

#[test]
fn t_coord_05_cursor_paging_has_no_duplicates_or_gaps_while_documents_are_added() {
    // Arrange: 촬영일 정렬(필드 정렬은 점수 통계의 영향을 받지 않는다), 동점 포함
    let dir = tempdir().unwrap();
    let docs: Vec<AssetDoc> = (1..=55)
        .map(|i| with(i, "p.jpg", |d| d.captured_at = Some(1000 - (i as i64 / 5))))
        .collect();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);
    let mut r = creq("");
    r.sort = sorted_by(SortField::CapturedAt, true);
    r.limit = 10;

    // Act: 페이지 사이에 새 문서(맨 앞·맨 뒤 정렬 위치)를 추가한다
    let mut seen: Vec<u64> = Vec::new();
    let mut pages = 0;
    loop {
        let page = search(&h, &r);
        seen.extend(&page.ids);
        pages += 1;
        if pages == 2 {
            idx.upsert(&with(900, "new.jpg", |d| d.captured_at = Some(5000)))
                .unwrap();
            idx.upsert(&with(901, "old.jpg", |d| d.captured_at = Some(1)))
                .unwrap();
            idx.commit().unwrap();
        }
        match page.next_cursor {
            Some(c) => r.cursor = Some(c),
            None => break,
        }
    }

    // Assert: 처음 있던 55건은 정확히 한 번씩, 앞쪽에 들어온 900 은 이미 지난 위치라 나오지 않는다
    let mut original: Vec<u64> = seen.iter().copied().filter(|i| *i <= 55).collect();
    let len = original.len();
    original.sort_unstable();
    original.dedup();
    assert_eq!(original.len(), 55, "no gaps");
    assert_eq!(len, 55, "no duplicates");
    assert!(!seen.contains(&900));
    assert!(
        seen.contains(&901),
        "a later-sorted new document is still reached"
    );
}

#[test]
fn t_coord_06_invalid_limit_and_cursors_are_rejected() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &(1..=30).map(|i| doc(i, "a.jpg")).collect::<Vec<_>>());
    let h = SearchHandle::for_index(&idx);
    let mut first = creq("");
    first.limit = 10;
    let cursor = search(&h, &first).next_cursor.unwrap();

    // Act + Assert: limit 범위
    for limit in [0, 201] {
        let mut r = creq("");
        r.limit = limit;
        assert!(
            matches!(h.search(&r), Err(SearchError::InvalidInput(_))),
            "limit {limit}"
        );
    }
    // 형식이 틀린 커서
    for bad in [
        "",
        "garbage",
        "c1.zz.1.2.3.4.5",
        "c1.1.1.1.1.1",
        "c1.1.1.1.1.1.0",
    ] {
        let mut r = first.clone();
        r.cursor = Some(bad.into());
        assert!(
            matches!(h.search(&r), Err(SearchError::InvalidInput(_))),
            "cursor {bad:?}"
        );
    }
    // 다른 질의·정렬에 쓴 커서 / 다른 세대의 커서
    let mut other = first.clone();
    other.cursor = Some(cursor.clone());
    other.sort = sorted_by(SortField::Size, true);
    assert!(matches!(h.search(&other), Err(SearchError::StaleCursor)));
    let other_handle = SearchHandle::for_index(&idx);
    let mut same = first.clone();
    same.cursor = Some(cursor);
    assert!(
        matches!(other_handle.search(&same), Err(SearchError::StaleCursor)),
        "generation differs"
    );
}

#[test]
fn t_coord_07_empty_query_without_filters_browses_all_and_empty_index_returns_zero() {
    // Arrange
    let dir = tempdir().unwrap();
    let empty = indexed(&dir, &[]);
    let h_empty = SearchHandle::for_index(&empty);
    let dir2 = tempdir().unwrap();
    let idx = indexed(&dir2, &[doc(1, "a.jpg"), doc(2, "b.jpg")]);
    let h = SearchHandle::for_index(&idx);

    // Act
    let none = search(&h_empty, &creq(""));
    let all = search(&h, &creq("   "));
    let no_match = search(&h, &creq("zzzz"));
    let handle_without_index = SearchHandle::new();

    // Assert
    assert_eq!((none.ids.len(), none.total), (0, 0));
    assert_eq!((all.ids, all.total), (vec![1, 2], 2));
    assert_eq!(no_match.total, 0);
    assert!(matches!(
        handle_without_index.search(&creq("")),
        Err(SearchError::NotReady)
    ));
}

#[test]
fn t_coord_08_range_filters_include_their_boundaries() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.mp4", |d| {
                d.file_size = 100;
                d.duration_ms = Some(1000);
                d.captured_at = Some(500);
            }),
            with(2, "b.mp4", |d| {
                d.file_size = 200;
                d.duration_ms = Some(2000);
                d.captured_at = Some(600);
            }),
            with(3, "c.mp4", |d| {
                d.file_size = 300;
                d.duration_ms = Some(3000);
                d.captured_at = Some(700);
            }),
            with(4, "d.mp4", |d| d.file_size = 150), // 길이·촬영일 없음
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let ids_of = |f: Filters| {
        let mut r = creq("");
        r.filters = f;
        let mut v = search(&h, &r).ids;
        v.sort_unstable();
        v
    };

    // Act + Assert
    let size = |min, max| Filters {
        file_size: Range { min, max },
        ..Default::default()
    };
    assert_eq!(
        ids_of(size(Some(200), Some(300))),
        [2, 3],
        "min and max inclusive"
    );
    assert_eq!(ids_of(size(Some(201), None)), [3]);
    assert_eq!(ids_of(size(None, Some(99))), Vec::<u64>::new());
    let dur = Filters {
        duration_ms: Range {
            min: Some(1000),
            max: Some(2000),
        },
        ..Default::default()
    };
    assert_eq!(
        ids_of(dur),
        [1, 2],
        "assets without duration never match a duration range"
    );
    let cap = Filters {
        captured_at: Range {
            min: Some(600),
            max: Some(600),
        },
        ..Default::default()
    };
    assert_eq!(ids_of(cap), [2]);
}

// 분류: Happy / Business rule(필터는 순위 전에 적용) / Boundary(0건·빈 질의·상한) / External(NotReady)
#[test]
fn t_coord_09_keyword_candidates_apply_filters_before_ranking_and_are_capped() {
    // Arrange: beach 가 이미지 3건·영상 1건, 영상 필터 적용
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "beach.jpg", |_| {}),
            with(2, "beach_a.jpg", |_| {}),
            with(3, "beach_b.jpg", |_| {}),
            with(4, "notes.mp4", |d| {
                d.media_type = MediaType::Video;
                d.user_memo = "beach".into();
            }),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let video = Filters {
        media_types: vec![MediaType::Video],
        ..Filters::default()
    };
    let b = FilenameStrategy::B;

    // Act
    let filtered = h.keyword_candidates("beach", b, &video, 1).unwrap();
    let capped = h
        .keyword_candidates("beach", b, &Filters::default(), 2)
        .unwrap();
    let empty_query = h
        .keyword_candidates("  ", b, &Filters::default(), 5)
        .unwrap();
    let zero = h.keyword_candidates("beach", b, &Filters::default(), 0);
    let too_many = h.keyword_candidates("beach", b, &Filters::default(), 10_001);

    // Assert: 상위 1건이어도 필터 밖 문서가 자리를 차지하지 않는다
    assert_eq!(filtered, [4]);
    assert_eq!(capped.len(), 2);
    assert!(empty_query.is_empty());
    assert_eq!(zero.unwrap(), Vec::<u64>::new());
    assert!(matches!(too_many, Err(SearchError::InvalidInput(_))));
}

#[test]
fn t_coord_10_filtered_asset_ids_returns_the_whole_allowed_set_and_not_ready_fails() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.jpg", |_| {}),
            with(2, "b.mp4", |d| d.media_type = MediaType::Video),
            with(3, "c.mp4", |d| d.media_type = MediaType::Video),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let videos = Filters {
        media_types: vec![MediaType::Video],
        ..Filters::default()
    };
    let none_online = Filters {
        online_volume_ids: Some(vec![]),
        ..Filters::default()
    };

    // Act
    let mut all = h.filtered_asset_ids(&Filters::default()).unwrap();
    let mut vids = h.filtered_asset_ids(&videos).unwrap();
    let nothing = h.filtered_asset_ids(&none_online).unwrap();
    let not_ready = SearchHandle::new().filtered_asset_ids(&Filters::default());

    // Assert
    all.sort_unstable();
    vids.sort_unstable();
    assert_eq!(all, [1, 2, 3]);
    assert_eq!(vids, [2, 3]);
    assert!(nothing.is_empty(), "no online volume = no results");
    assert!(matches!(not_ready, Err(SearchError::NotReady)));
}

// Time/Business rule: 색인 도장은 변경이 없으면 그대로이고, 커밋·삭제·재구축(세대)마다 바뀐다
#[test]
fn t_coord_11_index_stamp_is_stable_until_the_index_changes() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &[with(1, "a.jpg", |_| {}), with(2, "b.jpg", |_| {})]);
    let h = SearchHandle::for_index(&idx);
    let first = h.index_stamp().unwrap();

    // Act + Assert: 읽기만으로는 그대로, 결과 도장도 같다
    assert_eq!(h.index_stamp().unwrap(), first);
    let (stamp, ids) = h.filtered_asset_ids_stamped(&Filters::default()).unwrap();
    assert_eq!(stamp, first);
    assert_eq!(ids.len(), 2);

    // Act: 문서 추가 커밋 -> 도장이 바뀐다 (리더가 새 커밋을 볼 때까지 잠깐 기다린다)
    idx.upsert(&with(3, "c.jpg", |_| {})).unwrap();
    idx.commit().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.index_stamp().unwrap() == first {
        assert!(std::time::Instant::now() < deadline, "stamp never changed");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (after, ids) = h.filtered_asset_ids_stamped(&Filters::default()).unwrap();
    assert_ne!(after, first);
    assert_eq!(ids.len(), 3);

    // Assert: 색인이 없으면 NotReady
    assert!(matches!(
        SearchHandle::new().index_stamp(),
        Err(SearchError::NotReady)
    ));
}

// Business rule: 새 수집기는 기준 구현(DocSetCollector)과 같은 집합을 낸다 — 삭제된 문서·세그먼트 여러 개·빈 결과 포함
#[test]
fn t_coord_12_asset_id_collector_matches_the_docset_reference_including_deletes() {
    // Arrange: 커밋을 나눠 세그먼트를 여러 개로, 그중 일부는 삭제
    let dir = tempdir().unwrap();
    let docs: Vec<AssetDoc> = (1..=30)
        .map(|i| {
            with(i, "p.jpg", |d| {
                if i % 3 == 0 {
                    d.media_type = MediaType::Video;
                }
            })
        })
        .collect();
    let idx = indexed(&dir, &docs[..10]);
    for chunk in docs[10..].chunks(10) {
        for d in chunk {
            idx.upsert(d).unwrap();
        }
        idx.commit().unwrap();
    }
    idx.delete(4).unwrap();
    idx.delete(25).unwrap();
    idx.commit().unwrap();
    let h = SearchHandle::for_index(&idx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while h.filtered_asset_ids(&Filters::default()).unwrap().len() != 28 {
        assert!(
            std::time::Instant::now() < deadline,
            "reader did not reload"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let videos = Filters {
        media_types: vec![MediaType::Video],
        ..Filters::default()
    };
    let none = Filters {
        online_volume_ids: Some(vec![]),
        ..Filters::default()
    };

    for f in [Filters::default(), videos, none] {
        // Act
        let mut fast = h.filtered_asset_ids(&f).unwrap();
        let mut reference = h.filtered_asset_ids_docset(&f).unwrap();
        fast.sort_unstable();
        reference.sort_unstable();

        // Assert
        assert_eq!(fast, reference);
        assert!(
            !fast.contains(&4) && !fast.contains(&25),
            "deleted docs are skipped"
        );
    }
}

#[test]
fn t_coord_title_01_title_sort_uses_the_title_and_falls_back_to_the_file_name() {
    // Arrange: 이름순과 제목순이 달라지는 구성 (3 은 제목 없음 -> 파일명, 2 는 공백뿐인 제목 -> 파일명)
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "a.jpg", |d| d.title = "zebra".into()),
            with(2, "b.jpg", |d| d.title = "   ".into()),
            with(3, "c.jpg", |_| {}),
            with(4, "d.jpg", |d| d.title = "가을".into()),
        ],
    );
    let h = SearchHandle::for_index(&idx);
    let order = |f, desc| {
        let mut r = creq("");
        r.sort = sorted_by(f, desc);
        search(&h, &r).ids
    };

    // Act + Assert
    assert_eq!(order(SortField::Name, false), [1, 2, 3, 4], "파일명 기준");
    assert_eq!(
        order(SortField::Title, false),
        [2, 3, 1, 4],
        "b.jpg, c.jpg, zebra, 가을"
    );
    assert_eq!(order(SortField::Title, true), [4, 1, 3, 2]);
}

#[test]
fn t_coord_title_02_title_is_keyword_searchable_and_cursor_does_not_cross_sort_fields() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            with(1, "IMG_1.jpg", |d| d.title = "제주 일출".into()),
            with(2, "IMG_2.jpg", |_| {}),
            with(3, "IMG_3.jpg", |_| {}),
        ],
    );
    let h = SearchHandle::for_index(&idx);

    // Act
    let mut on = creq("일출");
    on.filters.search_title = true;
    let by_title = search(&h, &on);
    let hidden = search(&h, &creq("일출"));
    let mut file_on = creq("img");
    file_on.filters.search_title = true;
    let file_off = creq("img");
    let mut first = creq("");
    first.sort = sorted_by(SortField::Title, false);
    first.limit = 1;
    let page = search(&h, &first);
    let mut other = first.clone();
    other.sort = sorted_by(SortField::Name, false);
    other.cursor = page.next_cursor.clone();
    let err = h.search(&other).unwrap_err();

    // Assert: 제목 검색 / 정렬 필드가 다른 요청에 커서를 재사용하면 StaleCursor (이름순 <-> 제목순 토글 시 안전)
    assert_eq!(by_title.ids, [1]);
    assert!(hidden.ids.is_empty(), "disabled: stored title must not match");
    assert_eq!(search(&h, &file_on).ids.len(), 3, "file name matches when enabled");
    assert_eq!(search(&h, &file_off).ids.len(), 3, "file name matches when disabled");
    assert!(matches!(err, SearchError::StaleCursor), "{err:?}");
}
