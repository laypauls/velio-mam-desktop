//! 분석기·전처리 테스트.
//! 분류: T-fts-01 Happy / T-fts-02 Boundary / T-fts-03 Happy / T-fts-04 Boundary(+Validation)

mod common;

use common::*;
use mam_search::FilenameStrategy::{A, B, C};
use mam_search::analyzer::{Kind, tokenize};
use mam_search::preprocess::{ngrams, split_extension, split_pieces};
use mam_search::{IndexOptions, SearchIndex};
use tempfile::tempdir;

fn pieces(s: &str) -> Vec<String> {
    split_pieces(s).into_iter().map(|p| p.text).collect()
}

fn toks(kind: Kind, s: &str) -> Vec<String> {
    tokenize(kind, s)
        .unwrap()
        .into_iter()
        .map(|t| t.0)
        .collect()
}

/// 한글 음절을 조합형 자모(NFD)로 분해한다 (테스트 입력용).
fn nfd(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let s = c as u32;
            if !(0xAC00..=0xD7A3).contains(&s) {
                return vec![c];
            }
            let i = s - 0xAC00;
            let mut v = vec![
                char::from_u32(0x1100 + i / 588).unwrap(),
                char::from_u32(0x1161 + (i % 588) / 28).unwrap(),
            ];
            if !i.is_multiple_of(28) {
                v.push(char::from_u32(0x11A7 + i % 28).unwrap());
            }
            v
        })
        .collect()
}

#[test]
fn t_fts_01_filename_preprocess_splits_and_extension() {
    let (stem, ext) = split_extension("IMG_20240815_부산여행.jpg");
    assert_eq!(ext.as_deref(), Some("jpg"));
    assert_eq!(pieces(&stem), ["img", "20240815", "부산여행"]);

    assert_eq!(
        pieces("my-trip.final v2"),
        ["my", "trip", "final", "v", "2"]
    );
    assert_eq!(pieces("myTripPhoto"), ["my", "trip", "photo"]);
    assert_eq!(pieces("HTTPServer"), ["http", "server"]);
    assert_eq!(pieces("Trip2024"), ["trip", "2024"]);
    assert_eq!(pieces("부산trip2024"), ["부산", "trip", "2024"]);
    // 확장자 규칙: 숫자뿐인 접미사는 확장자가 아니다, 마지막 `.` 만 본다
    assert_eq!(split_extension("v1.0"), ("v1.0".to_string(), None));
    assert_eq!(
        split_extension("a.tar.GZ"),
        ("a.tar".to_string(), Some("gz".into()))
    );
    // 파일명 분석기(A)는 한글 조각만 형태소 분석한다
    assert_eq!(
        toks(Kind::Filename, "IMG_20240815_부산여행"),
        ["img", "20240815", "부산", "여행"]
    );
}

#[test]
fn t_fts_02_filename_boundaries() {
    // 빈 값·구분자만·확장자만
    assert!(pieces("").is_empty());
    assert!(pieces("___---...").is_empty());
    assert_eq!(split_extension(".jpg"), (String::new(), Some("jpg".into())));
    assert_eq!(split_extension(""), (String::new(), None));
    // 1글자
    assert_eq!(pieces("a"), ["a"]);
    assert!(ngrams("a").is_empty());
    // 255자 파일명
    let long = format!("{}.jpg", "가".repeat(251));
    let (stem, ext) = split_extension(&long);
    assert_eq!(ext.as_deref(), Some("jpg"));
    assert_eq!(pieces(&stem).len(), 1);
    assert_eq!(ngrams(&"가".repeat(251)).len(), 250 + 249);
    // 이모지는 구분자, 전각 영문·숫자는 반각으로
    assert_eq!(pieces("🎉party🎉사진"), ["party", "사진"]);
    assert_eq!(pieces("ＩＭＧ１２３"), ["img", "123"]);
    // NFD 한글은 정규화되어 같은 조각이 된다
    let decomposed = nfd("부산여행");
    assert_ne!(decomposed, "부산여행");
    assert_eq!(pieces(&decomposed), ["부산여행"]);
    // n-gram 은 바이트가 아니라 문자 기준
    assert_eq!(
        ngrams("부산여행"),
        ["부산", "산여", "여행", "부산여", "산여행"]
    );
}

#[test]
fn t_fts_02b_odd_filenames_index_and_search_without_panic() {
    let dir = tempdir().unwrap();
    let decomposed = format!("{}.jpg", nfd("부산여행"));
    let idx = indexed(
        &dir,
        &[
            doc(1, &decomposed),
            doc(2, ""),
            doc(3, ".jpg"),
            doc(4, "🎉.png"),
        ],
    );
    assert_eq!(ids(&idx, &req("부산", A)), [1]);
    assert_eq!(ids(&idx, &req("부산여행.jpg", A)), [1]);
    assert_eq!(ids(&idx, &req("jpg", A)), [1, 3]);
    assert_eq!(ids(&idx, &req("png", A)), [4]);
}

#[test]
fn t_fts_03_korean_particle_variants_match_same_doc() {
    let dir = tempdir().unwrap();
    let mut d1 = doc(1, "a.jpg");
    d1.ai_description = "강아지가 공원에서 달린다".into();
    let mut d2 = doc(2, "b.jpg");
    d2.user_tags = vec!["강아지".into()];
    let mut d3 = doc(3, "c.jpg");
    d3.ai_description = "고양이가 잔다".into();
    let idx = indexed(&dir, &[d1, d2, d3]);

    for q in ["강아지가", "강아지를", "강아지와", "강아지"] {
        assert_eq!(ids(&idx, &req(q, A)), [1, 2], "query {q}");
    }
    assert_eq!(ids(&idx, &req("고양이를", A)), [3]);
    // 조사뿐인 질의는 어떤 문서와도 일치하지 않는다 (색인 쪽에서 조사가 제거됨)
    assert!(ids(&idx, &req("를", A)).is_empty());
    // 영문 소문자화
    assert_eq!(toks(Kind::Korean, "Hello 강아지를"), ["hello", "강아지"]);
}

#[test]
fn t_fts_04_partial_match_strategies_and_one_char_policy() {
    // `여행사진` 은 ko-dic 에서 한 덩어리(NNP)라 A 로는 `사진` 을 못 찾는다
    let dir = tempdir().unwrap();
    let idx = indexed(
        &dir,
        &[
            doc(1, "여행사진_01.jpg"),
            doc(2, "IMG_20240815_부산여행.jpg"),
            doc(3, "trip_photo.png"),
            doc(4, "a_b.jpg"),
        ],
    );
    // A: 형태소 경계로만
    assert!(ids(&idx, &req("사진", A)).is_empty());
    assert_eq!(ids(&idx, &req("여행", A)), [2]);
    // B: n-gram — 2음절 한국어·영문·숫자 부분 문자열
    assert_eq!(ids(&idx, &req("사진", B)), [1]);
    assert_eq!(ids(&idx, &req("부산", B)), [2]);
    assert_eq!(ids(&idx, &req("rip", B)), [3]);
    assert_eq!(ids(&idx, &req("0815", B)), [2]);
    assert_eq!(ids(&idx, &req("행사진", B)), [1]);
    // C: 정규식 `.*조각.*`
    assert_eq!(ids(&idx, &req("사진", C)), [1]);
    assert_eq!(ids(&idx, &req("rip", C)), [3]);
    assert_eq!(ids(&idx, &req("0815", C)), [2]);
    // 정확 파일명 검색은 모든 전략에서 동작
    for s in [A, B, C] {
        assert_eq!(ids(&idx, &req("IMG_20240815_부산여행.jpg", s)), [2]);
    }
    // 1글자 정책: 부분 일치 없음 — 토큰 전체가 1글자일 때만 일치
    for s in [A, B, C] {
        assert_eq!(ids(&idx, &req("a", s)), [4], "strategy {s:?}");
        assert!(ids(&idx, &req("r", s)).is_empty(), "strategy {s:?}");
        assert!(ids(&idx, &req("사", s)).is_empty(), "strategy {s:?}");
    }
}

#[test]
fn t_fts_04b_strategy_requires_matching_index_build() {
    // Validation: A 전용 인덱스에서 B·C 질의는 조용한 빈 결과가 아니라 오류
    let dir = tempdir().unwrap();
    let idx =
        SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::strategy_a()))
            .unwrap();
    assert!(idx.search(&req("사진", B)).is_err());
    assert!(idx.search(&req("사진", C)).is_err());
    assert!(idx.search(&req("사진", A)).is_ok());
}
