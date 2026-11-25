//! 사용자 정의 메타데이터 필드 필터 — 임시 디렉터리의 실제 Tantivy 인덱스 (작업 문서 step 4).
//! 분류: T-umf-01 Happy+Boundary(keyword) / 02 number / 03 date / 04 boolean / 05 enum / 06 multi_enum /
//! 07 Has·Missing / 08 Business rule(필터 조합) / 09 Validation(구조 거부) / 10 Business rule(후보·id 수집 경로) /
//! 11 Business rule(커서 지문) / 12 Happy(keyword 검색) / 13 Boundary(긴 keyword)

mod common;

use common::{doc, open_all, req};
use mam_search::coordinator::{CoordRequest, Direction, SearchHandle, SortSpec};
use mam_search::{
    AssetDoc, FilenameStrategy, Filters, MediaType, SearchError, SearchIndex, UserFieldCondition as C,
    UserFieldValue, UserValue as V,
};
use tempfile::{TempDir, tempdir};

const KW: u64 = 1;
const NUM: u64 = 2;
const DATE: u64 = 3;
const BOOL: u64 = 4;
const ENUM: u64 = 5;
const MULTI: u64 = 6;

fn uf(field_id: u64, value: V) -> UserFieldValue {
    UserFieldValue { field_id, value }
}

fn with(id: u64, name: &str, vals: Vec<UserFieldValue>) -> AssetDoc {
    let mut d = doc(id, name);
    d.user_fields = vals;
    d
}

fn multi(field_id: u64, options: &[i64]) -> Vec<UserFieldValue> {
    options.iter().map(|o| uf(field_id, V::Int(*o))).collect()
}

/// 자산 1..=8. 4 번은 사용자 필드 값이 하나도 없다.
fn dataset() -> Vec<AssetDoc> {
    let mut d2 = with(
        2,
        "b.mp4",
        [
            vec![
                uf(KW, V::Keyword("beta".into())),
                uf(NUM, V::Number(20.5)),
                uf(DATE, V::Int(20261115)),
                uf(BOOL, V::Int(0)),
                uf(ENUM, V::Int(101)),
            ],
            multi(MULTI, &[201]),
        ]
        .concat(),
    );
    d2.media_type = MediaType::Video;
    vec![
        with(
            1,
            "a.jpg",
            [
                vec![
                    uf(KW, V::Keyword("Alpha Beam".into())),
                    uf(NUM, V::Number(10.0)),
                    uf(DATE, V::Int(20261101)),
                    uf(BOOL, V::Int(1)),
                    uf(ENUM, V::Int(100)),
                ],
                multi(MULTI, &[200, 201]),
            ]
            .concat(),
        ),
        d2,
        with(3, "c.jpg", vec![uf(KW, V::Keyword("ALPHA BEAM".into()))]),
        with(4, "d.jpg", vec![]),
        with(
            5,
            "e.jpg",
            [
                vec![
                    uf(KW, V::Keyword("Cafe\u{301}".into())), // NFD
                    uf(NUM, V::Number(1e300)),
                    uf(DATE, V::Int(20261130)),
                    uf(ENUM, V::Int(100)),
                ],
                multi(MULTI, &[200, 201, 202]),
            ]
            .concat(),
        ),
        with(6, "f.jpg", vec![uf(NUM, V::Number(-5.5)), uf(DATE, V::Int(20260101))]),
        with(7, "g.jpg", vec![uf(NUM, V::Number(-0.0))]),
        with(8, "h.jpg", vec![uf(NUM, V::Number(3.0))]),
    ]
}

/// `per_doc_segment` = 문서마다 commit (세그먼트마다 숫자 열 타입이 달라질 수 있다 — 정수값만 있는 세그먼트 등).
fn build(dir: &TempDir, docs: &[AssetDoc], per_doc_segment: bool) -> SearchIndex {
    let idx = open_all(dir);
    for d in docs {
        idx.upsert(d).unwrap();
        if per_doc_segment {
            idx.commit().unwrap();
        }
    }
    idx.commit().unwrap();
    idx
}

fn found(idx: &SearchIndex, conds: Vec<C>) -> Vec<u64> {
    let mut r = req("", FilenameStrategy::A);
    r.filters.user_fields = conds;
    common::ids(idx, &r)
}

/// 두 세그먼트 배치(문서별 세그먼트 / 단일 세그먼트) 모두에서 같은 결과여야 한다.
fn both_layouts(check: impl Fn(&SearchIndex)) {
    for per_doc in [false, true] {
        let dir = tempdir().unwrap();
        check(&build(&dir, &dataset(), per_doc));
    }
}

fn num(min: Option<f64>, max: Option<f64>) -> C {
    C::NumberRange { field_id: NUM, min, max }
}

fn date(min: Option<i64>, max: Option<i64>) -> C {
    C::DateRange { field_id: DATE, min, max }
}

#[test]
fn t_umf_01_keyword_matches_case_and_nfc_insensitively_as_a_whole_value() {
    both_layouts(|idx| {
        let kw = |vs: &[&str]| C::KeywordAnyOf {
            field_id: KW,
            values: vs.iter().map(|s| s.to_string()).collect(),
        };
        assert_eq!(found(idx, vec![kw(&["alpha beam"])]), [1, 3], "case-insensitive");
        assert_eq!(found(idx, vec![kw(&["  ALPHA BEAM "])]), [1, 3], "trimmed");
        assert_eq!(found(idx, vec![kw(&["ALPHA BEAM", "Beta"])]), [1, 2, 3], "any of");
        assert_eq!(found(idx, vec![kw(&["Caf\u{e9}"])]), [5], "NFC query vs NFD value");
        assert_eq!(found(idx, vec![kw(&["alpha"])]), Vec::<u64>::new(), "whole value, not a token");
        assert_eq!(found(idx, vec![kw(&["nothing"])]), Vec::<u64>::new());
        // 다른 필드의 같은 값에는 걸리지 않는다
        let other = C::KeywordAnyOf { field_id: 99, values: vec!["beta".into()] };
        assert_eq!(found(idx, vec![other]), Vec::<u64>::new());
    });
}

#[test]
fn t_umf_02_number_range_is_inclusive_and_exact_match_works_for_integer_looking_values() {
    both_layouts(|idx| {
        assert_eq!(found(idx, vec![num(Some(10.0), Some(20.5))]), [1, 2], "both ends inclusive");
        assert_eq!(found(idx, vec![num(Some(10.0), None)]), [1, 2, 5], "open max");
        assert_eq!(found(idx, vec![num(None, Some(3.0))]), [6, 7, 8], "open min, inclusive max");
        assert_eq!(found(idx, vec![num(Some(3.0), Some(3.0))]), [8], "exact match of 3.0 via [x, x]");
        assert_eq!(found(idx, vec![num(Some(10.0), Some(10.0))]), [1]);
        assert_eq!(found(idx, vec![num(Some(0.0), Some(0.0))]), [7], "-0.0 value matches 0.0 query");
        assert_eq!(found(idx, vec![num(Some(-0.0), Some(-0.0))]), [7], "-0.0 query is normalized");
        assert_eq!(found(idx, vec![num(Some(-5.5), Some(-5.5))]), [6], "negative exact");
        assert_eq!(found(idx, vec![num(Some(4.0), Some(9.0))]), Vec::<u64>::new(), "empty range");
        assert_eq!(found(idx, vec![num(Some(1e299), None)]), [5], "very large value");
        assert_eq!(found(idx, vec![num(None, None)]), [1, 2, 5, 6, 7, 8], "unbounded = any number");
    });
}

#[test]
fn t_umf_03_date_range_is_inclusive() {
    both_layouts(|idx| {
        assert_eq!(found(idx, vec![date(Some(20261101), Some(20261115))]), [1, 2]);
        assert_eq!(found(idx, vec![date(Some(20261115), None)]), [2, 5]);
        assert_eq!(found(idx, vec![date(None, Some(20260101))]), [6]);
        assert_eq!(found(idx, vec![date(Some(20261115), Some(20261115))]), [2], "single day");
        assert_eq!(found(idx, vec![date(Some(20261116), Some(20261129))]), Vec::<u64>::new());
        assert_eq!(found(idx, vec![date(None, None)]), [1, 2, 5, 6], "unbounded = any date");
        // date 필드 번호가 아닌 필드에는 걸리지 않는다
        let other = C::DateRange { field_id: 98, min: None, max: None };
        assert_eq!(found(idx, vec![other]), Vec::<u64>::new());
    });
}

#[test]
fn t_umf_04_boolean_true_and_false_are_distinct() {
    both_layouts(|idx| {
        assert_eq!(found(idx, vec![C::Bool { field_id: BOOL, value: true }]), [1]);
        assert_eq!(found(idx, vec![C::Bool { field_id: BOOL, value: false }]), [2]);
    });
}

#[test]
fn t_umf_05_enum_matches_any_of_the_selected_options() {
    both_layouts(|idx| {
        let e = |ids: &[u64]| C::EnumAnyOf { field_id: ENUM, option_ids: ids.to_vec() };
        assert_eq!(found(idx, vec![e(&[100])]), [1, 5]);
        assert_eq!(found(idx, vec![e(&[100, 101])]), [1, 2, 5]);
        assert_eq!(found(idx, vec![e(&[999])]), Vec::<u64>::new());
        // 같은 option id 라도 다른 필드에서는 다른 값이다
        assert_eq!(found(idx, vec![C::EnumAnyOf { field_id: MULTI, option_ids: vec![100] }]), Vec::<u64>::new());
    });
}

#[test]
fn t_umf_06_multi_enum_any_vs_all() {
    both_layouts(|idx| {
        let m = |ids: &[u64], all| C::MultiEnum { field_id: MULTI, option_ids: ids.to_vec(), all };
        assert_eq!(found(idx, vec![m(&[200], false)]), [1, 5]);
        assert_eq!(found(idx, vec![m(&[202, 201], false)]), [1, 2, 5], "any");
        assert_eq!(found(idx, vec![m(&[200, 201], true)]), [1, 5], "all");
        assert_eq!(found(idx, vec![m(&[201, 202], true)]), [5]);
        assert_eq!(found(idx, vec![m(&[200, 201, 202], true)]), [5]);
        assert_eq!(found(idx, vec![m(&[202, 200, 999], true)]), Vec::<u64>::new(), "one missing option");
        assert_eq!(found(idx, vec![m(&[201], true)]), [1, 2, 5], "all of one = any of one");
    });
}

#[test]
fn t_umf_07_has_and_missing_are_complements() {
    both_layouts(|idx| {
        assert_eq!(found(idx, vec![C::Has { field_id: KW }]), [1, 2, 3, 5]);
        assert_eq!(found(idx, vec![C::Missing { field_id: KW }]), [4, 6, 7, 8]);
        assert_eq!(found(idx, vec![C::Has { field_id: MULTI }]), [1, 2, 5]);
        assert_eq!(found(idx, vec![C::Missing { field_id: MULTI }]), [3, 4, 6, 7, 8]);
        // 정의된 적 없는 필드: 값 있음 = 없음, 값 없음 = 전부 (서비스 계층이 정의로 거부한다)
        assert_eq!(found(idx, vec![C::Has { field_id: 99 }]), Vec::<u64>::new());
        assert_eq!(found(idx, vec![C::Missing { field_id: 99 }]), [1, 2, 3, 4, 5, 6, 7, 8]);
        // Missing 두 개 + Has 하나
        assert_eq!(
            found(idx, vec![C::Missing { field_id: KW }, C::Missing { field_id: NUM }]),
            [4]
        );
        assert_eq!(
            found(idx, vec![C::Missing { field_id: KW }, C::Has { field_id: NUM }]),
            [6, 7, 8]
        );
    });
}

#[test]
fn t_umf_08_user_conditions_combine_with_each_other_and_with_fixed_filters() {
    both_layouts(|idx| {
        // 사용자 조건끼리 AND
        assert_eq!(
            found(idx, vec![date(Some(20261101), Some(20261130)), C::Bool { field_id: BOOL, value: true }]),
            [1]
        );
        // 고정 필터(미디어 종류)와 AND — 사전 필터라 total 도 정확하다
        let mut r = req("", FilenameStrategy::A);
        r.filters.media_types = vec![MediaType::Video];
        r.filters.user_fields = vec![C::Has { field_id: NUM }];
        assert_eq!(common::ids(idx, &r), [2]);
        r.filters.media_types = vec![MediaType::Image];
        assert_eq!(common::ids(idx, &r), [1, 5, 6, 7, 8]);
        // 키워드 검색과 AND
        let mut r = req("alpha", FilenameStrategy::A);
        r.filters.user_fields = vec![C::Missing { field_id: NUM }];
        assert_eq!(common::ids(idx, &r), [3], "keyword hit narrowed by 'no number'");
        // 같은 필드에 조건 둘 = 교집합
        assert_eq!(found(idx, vec![num(Some(0.0), None), num(None, Some(10.0))]), [1, 7, 8]);
    });
}

#[test]
fn t_umf_09_structurally_invalid_conditions_are_rejected_on_every_query_path() {
    let dir = tempdir().unwrap();
    let idx = build(&dir, &dataset(), false);
    let h = SearchHandle::for_index(&idx);
    let invalid: Vec<(&str, C)> = vec![
        ("empty keywords", C::KeywordAnyOf { field_id: KW, values: vec![] }),
        ("blank keyword", C::KeywordAnyOf { field_id: KW, values: vec!["a".into(), "   ".into()] }),
        ("nan min", num(Some(f64::NAN), None)),
        ("nan max", num(None, Some(f64::NAN))),
        ("inf max", num(None, Some(f64::INFINITY))),
        ("neg inf min", num(Some(f64::NEG_INFINITY), None)),
        ("number min>max", num(Some(2.0), Some(1.0))),
        ("date min>max", date(Some(20261102), Some(20261101))),
        ("empty enum", C::EnumAnyOf { field_id: ENUM, option_ids: vec![] }),
        ("empty multi", C::MultiEnum { field_id: MULTI, option_ids: vec![], all: true }),
        ("option id > i64::MAX", C::EnumAnyOf { field_id: ENUM, option_ids: vec![u64::MAX] }),
    ];
    for (what, cond) in invalid {
        // 이후 경로는 모두 build_query 를 거친다: 직접 검색 / 조정기 / id 수집 / 하이브리드 후보
        let mut r = req("", FilenameStrategy::A);
        r.filters.user_fields = vec![cond.clone()];
        let direct = idx.search(&r);
        let coord = h.search(&CoordRequest {
            text: String::new(),
            strategy: FilenameStrategy::A,
            filters: r.filters.clone(),
            sort: SortSpec::default(),
            limit: 10,
            cursor: None,
            direction: Direction::Forward,
            at: None,
            collection_order: None,
        });
        let ids = h.filtered_asset_ids(&r.filters);
        let cands = h.keyword_candidates("a", FilenameStrategy::A, &r.filters, 5);
        for (path, res) in [
            ("search", direct.map(|_| ()).err()),
            ("coordinator", coord.map(|_| ()).err()),
            ("filtered_asset_ids", ids.map(|_| ()).err()),
            ("keyword_candidates", cands.map(|_| ()).err()),
        ] {
            assert!(
                matches!(res, Some(SearchError::InvalidInput(_))),
                "{what} via {path}: {res:?}"
            );
        }
    }
}

#[test]
fn t_umf_10_id_collection_and_hybrid_candidates_apply_the_same_conditions() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = build(&dir, &dataset(), true);
    let h = SearchHandle::for_index(&idx);
    let filters = Filters {
        user_fields: vec![C::Bool { field_id: BOOL, value: true }],
        ..Filters::default()
    };

    // Act / Assert: 후필터가 아니라 검색 단계에서 걸린다 — 상위 N=1 만 요청해도 조건에 맞는 자산이 나온다
    let mut ids = h.filtered_asset_ids(&filters).unwrap();
    ids.sort_unstable();
    assert_eq!(ids, [1]);
    let missing = Filters { user_fields: vec![C::Missing { field_id: KW }], ..Filters::default() };
    let mut ids = h.filtered_asset_ids(&missing).unwrap();
    ids.sort_unstable();
    assert_eq!(ids, [4, 6, 7, 8]);
    let docset = h.filtered_asset_ids_docset(&missing).unwrap().len();
    assert_eq!(docset, 4, "reference collector agrees");
    // 'a.jpg' 는 모든 파일명에 있지만 조건에 맞는 것만 후보가 된다
    let cands = h
        .keyword_candidates("jpg", FilenameStrategy::A, &Filters {
            user_fields: vec![C::Has { field_id: DATE }],
            ..Filters::default()
        }, 1)
        .unwrap();
    assert_eq!(cands.len(), 1);
    assert!([1, 5, 6].contains(&cands[0]), "{cands:?}");
}

#[test]
fn t_umf_11_changing_a_user_condition_makes_the_cursor_stale() {
    // Arrange: 값 있음 자산이 3건 이상이라 첫 페이지(2건)에 다음 커서가 생긴다
    let dir = tempdir().unwrap();
    let idx = build(&dir, &dataset(), false);
    let h = SearchHandle::for_index(&idx);
    let page = |filters: Filters, cursor: Option<String>| {
        h.search(&CoordRequest {
            text: String::new(),
            strategy: FilenameStrategy::A,
            filters,
            sort: SortSpec::default(),
            limit: 2,
            cursor,
            direction: Direction::Forward,
            at: None,
            collection_order: None,
        })
    };
    let has_num = || Filters { user_fields: vec![C::Has { field_id: NUM }], ..Filters::default() };
    let first = page(has_num(), None).unwrap();
    let cursor = first.next_cursor.clone().expect("more pages");

    // Act / Assert: 같은 조건이면 이어진다
    let second = page(has_num(), Some(cursor.clone())).unwrap();
    assert!(!second.ids.is_empty());
    // 조건 값 변경 / 종류 변경 / 조건 추가·제거는 모두 stale
    let changed: Vec<Filters> = vec![
        Filters { user_fields: vec![C::Has { field_id: DATE }], ..Filters::default() },
        Filters { user_fields: vec![C::Missing { field_id: NUM }], ..Filters::default() },
        Filters { user_fields: vec![C::Has { field_id: NUM }, C::Has { field_id: DATE }], ..Filters::default() },
        Filters::default(),
        Filters { user_fields: vec![num(Some(0.0), None)], ..Filters::default() },
    ];
    for f in changed {
        let err = page(f.clone(), Some(cursor.clone())).unwrap_err();
        assert!(matches!(err, SearchError::StaleCursor), "{f:?}: {err:?}");
    }
    let a = page(Filters { user_fields: vec![num(Some(1.0), None)], ..Filters::default() }, None).unwrap();
    let cur = a.next_cursor.expect("more pages");
    let err = page(
        Filters { user_fields: vec![num(Some(2.0), None)], ..Filters::default() },
        Some(cur),
    )
    .unwrap_err();
    assert!(matches!(err, SearchError::StaleCursor), "bound change: {err:?}");
}

#[test]
fn t_umf_12_keyword_field_values_are_found_by_the_general_keyword_search() {
    // Arrange: 5 = 사용자 keyword 값에만 'beam', 9 = 태그에 'beam'
    let dir = tempdir().unwrap();
    let mut tagged = doc(9, "z.jpg");
    tagged.user_tags = vec!["beam".into()];
    let mut docs = dataset();
    docs.push(tagged);
    let idx = build(&dir, &docs, false);

    // Act
    let by_value = common::ids(&idx, &req("beam", FilenameStrategy::A));
    let hits = idx.search(&req("beam", FilenameStrategy::A)).unwrap();
    let by_prefix_word = common::ids(&idx, &req("alpha beam", FilenameStrategy::A));
    let number_value = common::ids(&idx, &req("10", FilenameStrategy::A));

    // Assert
    assert_eq!(by_value, [1, 3, 9]);
    assert_eq!(hits[0].asset_id, 9, "tag (boost 5.0) outranks a user keyword field value (boost 2.0)");
    assert_eq!(by_prefix_word, [1, 3]);
    assert!(!number_value.contains(&1), "number fields are not part of keyword search");
    // keyword 가 아닌 값(enum 의 option id 등)도 검색 텍스트에 들어가지 않는다
    assert_eq!(common::ids(&idx, &req("101", FilenameStrategy::A)), Vec::<u64>::new());
}

#[test]
fn t_umf_13_very_long_keyword_value_is_still_found_by_itself() {
    // Boundary: 7만 자 값은 Tantivy 가 term 을 버리므로 색인·질의 모두 같은 상한(200자)으로 자른다
    let dir = tempdir().unwrap();
    let long = "가나다".repeat(23_334);
    let idx = build(&dir, &[with(1, "x.jpg", vec![uf(KW, V::Keyword(long.clone()))])], false);

    let hit = found(&idx, vec![C::KeywordAnyOf { field_id: KW, values: vec![long] }]);

    assert_eq!(hit, [1]);
}
