//! 코드 리뷰 지적(C1, I1~I4, I7) 회귀 테스트.
//! 분류: T-fts-14 Validation / T-fts-15 Business rule / T-fts-16 Business rule /
//!       T-fts-17 External failure(+Validation) / T-fts-18 Boundary / T-fts-19 Happy

mod common;

use common::*;
use mam_search::FilenameStrategy::{A, B, C};
use mam_search::analyzer::{Kind, tokenize};
use mam_search::{IndexOptions, MAX_LIMIT, MAX_WINDOW, SearchError, SearchIndex, SearchRequest};
use tempfile::tempdir;

fn toks(kind: Kind, s: &str) -> Vec<String> {
    tokenize(kind, s)
        .unwrap()
        .into_iter()
        .map(|t| t.0)
        .collect()
}

#[test]
fn t_fts_14_limit_and_offset_are_validated_without_panic() {
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &[doc(1, "a.jpg")]);
    let bad = |limit: usize, offset: usize| {
        let r = SearchRequest {
            limit,
            offset,
            ..Default::default()
        };
        matches!(idx.search(&r), Err(SearchError::InvalidInput(_)))
    };
    assert!(bad(1 << 40, 0)); // 할당 중단 재현
    assert!(bad(MAX_LIMIT + 1, 0));
    assert!(bad(10, usize::MAX)); // 덧셈 오버플로 재현
    assert!(bad(MAX_LIMIT, usize::MAX - 5));
    assert!(bad(1, MAX_WINDOW)); // offset + limit > MAX_WINDOW
    // 경계값은 허용
    let ok = |limit: usize, offset: usize| {
        idx.search(&SearchRequest {
            limit,
            offset,
            ..Default::default()
        })
        .is_ok()
    };
    assert!(ok(MAX_LIMIT, 0));
    assert!(ok(1, MAX_WINDOW - 1));
    assert!(ok(MAX_LIMIT, MAX_WINDOW - MAX_LIMIT));
}

#[test]
fn t_fts_15_symbol_and_particle_only_words_are_dropped_from_query() {
    let dir = tempdir().unwrap();
    let mut d3 = doc(3, "c.jpg");
    d3.user_tags = vec!["가족".into(), "바다".into()];
    let idx = indexed(&dir, &[doc(1, "trip.jpg"), doc(2, "부산.jpg"), d3]);
    for s in [A, B, C] {
        assert_eq!(ids(&idx, &req("trip !!!", s)), [1], "{s:?}");
        assert_eq!(ids(&idx, &req("부산 에서", s)), [2], "{s:?}");
        assert_eq!(ids(&idx, &req("가족 & 바다", s)), [3], "{s:?}");
        // 쓸 단어가 하나도 없으면 전체가 아니라 빈 결과
        assert!(ids(&idx, &req("& !!!", s)).is_empty(), "{s:?}");
        assert!(ids(&idx, &req("에서", s)).is_empty(), "{s:?}");
    }
    // 질의 모드만 폴백이 없다: 색인용 분석은 기능어뿐인 입력을 그대로 둔다
    assert_eq!(toks(Kind::Korean, "에서"), ["에서"]);
    assert!(toks(Kind::KoreanQuery, "에서").is_empty());
}

#[test]
fn t_fts_16_extension_lookalike_words_still_match_by_name() {
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            doc(1, "Mr.Kim.jpg"),
            doc(2, "trip.final.png"),
            doc(3, "plain.jpg"),
            doc(4, "v1.0_release.mp4"),
        ],
    );
    for s in [A, B, C] {
        assert_eq!(ids(&idx, &req("Mr.Kim", s)), [1], "{s:?}");
        assert_eq!(ids(&idx, &req("trip.final", s)), [2], "{s:?}");
        assert_eq!(ids(&idx, &req("trip.final.png", s)), [2], "{s:?}");
        assert_eq!(ids(&idx, &req("v1.0", s)), [4], "{s:?}");
        // 진짜 확장자 질의는 그대로
        assert_eq!(ids(&idx, &req("plain.jpg", s)), [3], "{s:?}");
        assert!(ids(&idx, &req("plain.png", s)).is_empty(), "{s:?}");
    }
}

#[test]
fn t_fts_17_reopen_with_different_options_or_version_fails_explicitly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("idx");
    {
        let idx = SearchIndex::open_or_create(&path, small(IndexOptions::strategy_a())).unwrap();
        idx.upsert(&doc(1, "사진.jpg")).unwrap();
        idx.commit().unwrap();
    }
    // 같은 옵션은 재오픈 가능 (writer 자원 차이는 무관)
    drop(SearchIndex::open_or_create(&path, small(IndexOptions::strategy_a())).unwrap());
    // 전략 옵션이 다르면 조용한 0건 대신 명시적 오류
    for other in [
        IndexOptions::all(),
        IndexOptions::strategy_b(),
        IndexOptions::strategy_c(),
    ] {
        let r = SearchIndex::open_or_create(&path, small(other));
        assert!(matches!(r, Err(SearchError::IndexMismatch(_))));
    }
    // 분석기 버전이 다른 메타 / 손상된 메타
    let meta = path.join("mam-search.meta");
    let original = std::fs::read_to_string(&meta).unwrap();
    std::fs::write(
        &meta,
        original.replace("analyzer_version=", "analyzer_version=9"),
    )
    .unwrap();
    assert!(matches!(
        SearchIndex::open_or_create(&path, small(IndexOptions::strategy_a())),
        Err(SearchError::IndexMismatch(_))
    ));
    std::fs::write(&meta, "garbage").unwrap();
    assert!(matches!(
        SearchIndex::open_or_create(&path, small(IndexOptions::strategy_a())),
        Err(SearchError::IndexMismatch(_))
    ));
    // 메타가 없는 기존 인덱스도 재구축 대상
    std::fs::remove_file(&meta).unwrap();
    assert!(matches!(
        SearchIndex::open_or_create(&path, small(IndexOptions::strategy_a())),
        Err(SearchError::IndexMismatch(_))
    ));
    // 삭제 후 새 옵션으로 재구축
    std::fs::remove_dir_all(&path).unwrap();
    let idx = SearchIndex::open_or_create(&path, small(IndexOptions::all())).unwrap();
    idx.upsert(&doc(1, "사진.jpg")).unwrap();
    idx.commit().unwrap();
    assert_eq!(ids(&idx, &req("사진", B)), [1]);
}

#[test]
fn t_fts_18_very_long_hangul_filename_does_not_fail_partial_queries() {
    let dir = tempdir().unwrap();
    let long = "가나다라마바사아자차카타파하".repeat(18); // 252자 한글 조각
    let name = format!("{long}.jpg");
    assert!(long.chars().count() >= 250);
    let idx = indexed(&dir, &[doc(1, &name), doc(2, "부산여행사진.jpg")]);
    for s in [A, B, C] {
        // 긴 조각은 부분 일치 절을 건너뛰어도 질의 자체는 성공하고 정확 토큰으로 찾는다
        let r = idx.search(&req(&name, s));
        assert!(r.is_ok(), "{s:?}: {:?}", r.err());
        assert_eq!(ids(&idx, &req(&name, s)), [1], "{s:?}");
        // 긴 조각과 짧은 조각을 섞어도 성공
        assert!(idx.search(&req(&format!("{long} 사진"), s)).is_ok());
    }
    // 짧은 조각의 부분 일치는 영향받지 않는다
    assert_eq!(ids(&idx, &req("행사", C)), [2]);
}

#[test]
fn t_fts_19_inflected_verbs_reduce_to_stem() {
    // 어간 환원 전/후 (환원 전: 활용형 표면형이 그대로 색인되던 동작)
    assert_eq!(toks(Kind::Korean, "달린다"), ["달리"]);
    assert_eq!(toks(Kind::Korean, "달리다"), ["달리"]);
    assert_eq!(toks(Kind::Korean, "달리는"), ["달리"]);
    assert_eq!(toks(Kind::Korean, "했다"), ["하"]);
    assert_eq!(toks(Kind::Korean, "먹었다"), ["먹"]);
    assert_eq!(toks(Kind::Korean, "예쁜"), ["예쁘"]);
    assert_eq!(toks(Kind::Korean, "예쁘다"), ["예쁘"]);

    let dir = tempdir().unwrap();
    let mut d1 = doc(1, "a.jpg");
    d1.ai_description = "강아지가 공원에서 달린다".into();
    let mut d2 = doc(2, "b.jpg");
    d2.ai_description = "예쁜 꽃".into();
    let idx = indexed(&dir, &[d1, d2]);
    for q in ["달리다", "달린다", "달리는"] {
        assert_eq!(ids(&idx, &req(q, A)), [1], "{q}");
    }
    assert_eq!(ids(&idx, &req("예쁘다", A)), [2]);
    // 명사 복합어는 건드리지 않는다
    assert_eq!(toks(Kind::Korean, "여행사진"), ["여행사진"]);
}

#[test]
fn t_fts_22_trailing_single_syllable_particle_read_as_noun_is_dropped_in_query() {
    // ko-dic: `스키의` → 스키/NNG + 의/NNG (조사가 아니라 명사로 분석), `스키의 계절` 에서는 의/JKG
    let q = |s: &str| toks(Kind::KoreanQuery, s);
    assert_eq!(q("스키의"), ["스키"]);
    assert_eq!(q("사진의"), ["사진"]);
    assert_eq!(q("스키를"), ["스키"]);
    // 색인 쪽 토큰은 바뀌지 않는다 (ANALYZER_VERSION 유지)
    assert_eq!(toks(Kind::Korean, "스키의"), ["스키", "의"]);
    // 토큰이 하나뿐인 단어(`의` 단독은 명사로 분석됨)는 건드리지 않는다
    assert_eq!(q("의"), ["의"]);
    assert_eq!(q("스키장"), ["스키장"]);

    let dir = tempdir().unwrap();
    let mut d1 = doc(1, "a.jpg");
    d1.user_tags = vec!["스키".into()];
    let mut d2 = doc(2, "b.jpg");
    d2.ai_description = "스키를 타는 사람".into();
    let d3 = doc(3, "스키장.jpg");
    let idx = indexed(&dir, &[d1, d2, d3]);
    // `스키의` 는 형태소로 `스키` 를 찾는다. B·C 의 부분 일치는 조각 `스키의` 전체를 보므로
    // `스키장` 은 찾지 않고(알려진 한계 S1/S2), 질의 `스키` 일 때만 찾는다.
    for (s, want) in [(A, vec![1, 2]), (B, vec![1, 2, 3]), (C, vec![1, 2, 3])] {
        assert_eq!(ids(&idx, &req("스키의", s)), [1, 2], "{s:?}");
        assert_eq!(ids(&idx, &req("스키", s)), want, "{s:?}");
    }
}
