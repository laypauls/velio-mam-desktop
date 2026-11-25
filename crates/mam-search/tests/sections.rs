//! 구간 집계(`section_index`) 테스트 — 임시 디렉터리의 실제 Tantivy 인덱스.
//! 분류: T-sect-10 Happy+Time(월) / 11 Boundary(월 경계·시간대) / 12 Happy(이름 구간) / 13 Boundary(이름 가장자리) /
//! 14 Business rule(필터·질의·방향) / 15 Boundary(구간 없는 정렬·빈 결과) / 16 Happy(제목)

mod common;

use chrono::{FixedOffset, TimeZone};
use common::{doc, indexed};
use mam_search::AssetDoc;
use mam_search::coordinator::{
    CoordRequest, Direction, SearchHandle, SectionIndex, SectionKind, SortField, SortSpec,
    name_initial_bounds,
};
use mam_search::{FilenameStrategy, Filters, MediaType};
use tempfile::tempdir;
use unicode_normalization::UnicodeNormalization;

fn creq(sort: SortField, desc: bool) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters::default(),
        sort: SortSpec {
            field: sort,
            descending: desc,
        },
        limit: 200,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

fn utc(_: i64) -> i32 {
    0
}
fn kst(_: i64) -> i32 {
    9 * 3600
}

fn pairs(s: &SectionIndex) -> Vec<(String, u64)> {
    s.sections.iter().map(|x| (x.key.clone(), x.count)).collect()
}

/// 정렬 순서대로 모든 항목의 id (한 페이지 200건 이내인 데이터셋용)
fn order(h: &SearchHandle, r: &CoordRequest) -> Vec<u64> {
    h.search(r).unwrap().ids
}

/// 연속한 같은 키를 접어 (키, 개수) 로 — 정렬 순서와 구간 순서가 같은지 대조하는 기준
fn runs(keys: impl IntoIterator<Item = String>) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    for k in keys {
        match out.last_mut() {
            Some((last, n)) if *last == k => *n += 1,
            _ => out.push((k, 1)),
        }
    }
    out
}

fn month_of(secs: i64, offset: i32) -> String {
    let tz = FixedOffset::east_opt(offset).unwrap();
    tz.timestamp_opt(secs, 0).unwrap().format("%Y-%m").to_string()
}

#[test]
fn t_sect_10_month_sections_follow_sort_order_with_nulls_last() {
    // Arrange: 2023-12, 2024-01(여럿), 2024-03 (2월 비어 있음), 날짜 없음 2건
    let t = |y, m, d| chrono::Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap().timestamp();
    let times = [t(2023, 12, 31), t(2024, 1, 2), t(2024, 1, 15), t(2024, 1, 30), t(2024, 3, 1)];
    let mut docs: Vec<AssetDoc> = times
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut d = doc(i as u64 + 1, "m.jpg");
            d.captured_at = Some(*s);
            d
        })
        .collect();
    docs.push(doc(6, "n1.jpg"));
    docs.push(doc(7, "n2.jpg"));
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);

    for desc in [false, true] {
        // Act
        let r = creq(SortField::CapturedAt, desc);
        let s = h.section_index(&r, &utc).unwrap();

        // Assert
        let mut expect = vec![("2023-12", 1), ("2024-01", 3), ("2024-03", 1)];
        if desc {
            expect.reverse();
        }
        let mut expect: Vec<(String, u64)> = expect.into_iter().map(|(k, c)| (k.to_string(), c)).collect();
        expect.push(("none".into(), 2));
        assert_eq!(pairs(&s), expect, "desc={desc}");
        assert_eq!(s.kind, SectionKind::Month);
        assert_eq!(s.total, 7);
        assert_eq!(s.total, s.sections.iter().map(|x| x.count).sum::<u64>());
        assert_eq!(s.generation, h.generation());

        // 정렬된 실제 순서를 같은 규칙으로 접은 것과 같다
        let secs: std::collections::HashMap<u64, Option<i64>> =
            docs.iter().map(|d| (d.asset_id, d.captured_at)).collect();
        let sequence = runs(order(&h, &r).into_iter().map(|id| match secs[&id] {
            Some(v) => month_of(v, 0),
            None => "none".into(),
        }));
        assert_eq!(pairs(&s), sequence, "desc={desc}: sections == runs of the real order");
    }
}

#[test]
fn t_sect_11_month_boundaries_use_the_injected_utc_offset() {
    // Arrange: KST 2024-02-01 00:00 = 2024-01-31T15:00Z (2월 첫 순간) 과 1초 전
    let feb_first_kst = chrono::Utc.with_ymd_and_hms(2024, 1, 31, 15, 0, 0).unwrap().timestamp();
    let mut a = doc(1, "a.jpg");
    a.captured_at = Some(feb_first_kst);
    let mut b = doc(2, "b.jpg");
    b.captured_at = Some(feb_first_kst - 1);
    let mut c = doc(3, "c.jpg");
    c.captured_at = Some(feb_first_kst + 29 * 86_400 - 1); // 2024-02-29 23:59:59 KST (윤년 2월 마지막 초)
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &[a, b, c]);
    let h = SearchHandle::for_index(&idx);
    let r = creq(SortField::CapturedAt, false);

    // Act
    let in_kst = h.section_index(&r, &kst).unwrap();
    let in_utc = h.section_index(&r, &utc).unwrap();

    // Assert: 같은 3건이 시간대에 따라 다르게 나뉜다
    assert_eq!(pairs(&in_kst), [("2024-01".to_string(), 1), ("2024-02".to_string(), 2)]);
    assert_eq!(pairs(&in_utc), [("2024-01".to_string(), 2), ("2024-02".to_string(), 1)]);
}

#[test]
fn t_sect_11b_modified_at_is_milliseconds_and_has_no_null_bucket_in_practice() {
    // Arrange: 수정일은 ms 단위다
    let ms = |y, m, d| chrono::Utc.with_ymd_and_hms(y, m, d, 1, 0, 0).unwrap().timestamp_millis();
    let docs: Vec<AssetDoc> = [ms(2022, 5, 1), ms(2022, 5, 31), ms(2022, 6, 1)]
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let mut d = doc(i as u64 + 1, "x.jpg");
            d.modified_at = *v;
            d
        })
        .collect();
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);

    // Act
    let s = h.section_index(&creq(SortField::ModifiedAt, true), &utc).unwrap();

    // Assert
    assert_eq!(pairs(&s), [("2022-06".to_string(), 1), ("2022-05".to_string(), 2)]);
}

/// 테스트 쪽 독립 분류: 정렬 키(NFC + 소문자)의 첫 글자로 구간을 정한다.
fn expected_bucket(name: &str) -> &'static str {
    const CHO: [&str; 19] = [
        "ㄱ", "ㄱ", "ㄴ", "ㄷ", "ㄷ", "ㄹ", "ㅁ", "ㅂ", "ㅂ", "ㅅ", "ㅅ", "ㅇ", "ㅈ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ",
    ];
    let key: String = name.nfc().collect::<String>().to_lowercase();
    let Some(c) = key.chars().next() else { return "sym" };
    match c as u32 {
        0..=0x60 => "sym",
        0x61..=0x7a => ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z"][(c as u32 - 0x61) as usize],
        0x7b..=0xabff => "etc",
        cp @ 0xac00..=0xd7a3 => CHO[((cp - 0xac00) / 588) as usize],
        _ => "etc_end",
    }
}

fn name_docs(names: &[&str]) -> Vec<AssetDoc> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| doc(i as u64 + 1, n))
        .collect()
}

#[test]
fn t_sect_12_name_sections_cover_latin_symbols_accents_kana_and_hangul_in_sort_order() {
    // Arrange
    let names = [
        "Apple.jpg", "apricot.jpg", "banana.jpg", "_under.jpg", "1photo.jpg", "#hash.jpg", "-dash.jpg",
        "zebra.jpg", "Z-last.jpg", "{brace.jpg", "~tilde.jpg",
        "Éclair.jpg", "ümlaut.jpg", "ÉCLAIR2.jpg", "e\u{301}clair3.jpg", // NFC 후 é(U+00E9) → etc
        "カタカナ.jpg", "日本語.jpg", "ㄱ낱자.jpg", "ㅎ낱자.jpg", // 가나·한자·호환 자모 낱자 → etc
        "가방.jpg", "까치.jpg", "깋.jpg", "나비.jpg", "다람쥐.jpg", "따뜻.jpg", "라면.jpg", "마을.jpg", "바다.jpg", "빵.jpg",
        "사자.jpg", "싸움.jpg", "아기.jpg", "자전거.jpg", "짜장.jpg", "차.jpg", "카메라.jpg", "타조.jpg", "파도.jpg", "하늘.jpg", "힣.jpg",
        "\u{1100}\u{1161}\u{11a8}.jpg", // 조합형 가 → NFC 로 ㄱ
        "ퟀ.jpg", "ﬁle.jpg", "😀.jpg", "\u{ff21}fullwidth.jpg", // 힣 뒤 → etc_end
    ];
    let docs = name_docs(&names);
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);

    for desc in [false, true] {
        // Act
        let r = creq(SortField::Name, desc);
        let s = h.section_index(&r, &utc).unwrap();
        let real_order = order(&h, &r);

        // Assert
        assert_eq!(s.kind, SectionKind::Initial);
        assert_eq!(s.total, names.len() as u64);
        assert_eq!(s.sections.iter().map(|x| x.count).sum::<u64>(), s.total);
        let by_id: std::collections::HashMap<u64, &AssetDoc> = docs.iter().map(|d| (d.asset_id, d)).collect();
        let sequence = runs(real_order.iter().map(|id| expected_bucket(&by_id[id].file_name).to_string()));
        assert_eq!(pairs(&s), sequence, "desc={desc}: sections == runs of the real order");
        // 구간 키는 한 번씩만 나온다 (정렬이 구간 안에서 연속이라는 뜻)
        let mut keys: Vec<_> = s.sections.iter().map(|x| x.key.clone()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), s.sections.len(), "each section appears once");
    }

    // 몇 가지 구체적 기대값 (오름차순)
    let s = h.section_index(&creq(SortField::Name, false), &utc).unwrap();
    let get = |k: &str| s.sections.iter().find(|x| x.key == k).map_or(0, |x| x.count);
    assert_eq!((get("sym"), get("a"), get("z"), get("etc"), get("etc_end")), (4, 2, 2, 10, 4));
}

#[test]
fn t_sect_13_name_edge_rules() {
    // Arrange: 구간 경계 값 그대로
    let names = ["`tick.jpg", "a.jpg", "z.jpg", "z\u{10ffff}.jpg", "\u{ab00}.jpg", "가.jpg", "깋.jpg", "깍.jpg", "힣.jpg", "\u{d7a4}.jpg", ".hidden.jpg"];
    let docs = name_docs(&names);
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);

    // Act
    let s = h.section_index(&creq(SortField::Name, false), &utc).unwrap();

    // Assert: ` 와 . 는 a 앞(sym), U+AB00 은 가(U+AC00) 앞(etc), 가·깍(까 구간 안)은 ㄱ, 힣 은 ㅎ, U+D7A4 는 etc_end
    let got = pairs(&s);
    let expect: Vec<(String, u64)> = [("sym", 2), ("a", 1), ("z", 2), ("etc", 1), ("ㄱ", 3), ("ㅎ", 1), ("etc_end", 1)]
        .iter()
        .map(|(k, c)| (k.to_string(), *c))
        .collect();
    assert_eq!(got, expect);
    // 경계 표는 오름차순이며 라벨이 중복되지 않는다
    let bounds = name_initial_bounds();
    let mut labels: Vec<_> = bounds.iter().map(|b| b.0.clone()).collect();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), bounds.len());
}

#[test]
fn t_sect_14_sections_respect_filters_query_and_direction() {
    // Arrange: 영상은 3의 배수. 이름 "n{i%17}-photo{i}", 촬영일 월 분산
    let docs: Vec<AssetDoc> = (1..=60)
        .map(|i| {
            let mut d = doc(i, &format!("{}-photo{i}.jpg", ["a", "b", "c"][(i % 3) as usize]));
            d.captured_at = Some(1_700_000_000 + (i as i64 % 4) * 31 * 86_400);
            if i % 3 == 0 {
                d.media_type = MediaType::Video;
            }
            d
        })
        .collect();
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &docs);
    let h = SearchHandle::for_index(&idx);
    let mut r = creq(SortField::CapturedAt, true);
    r.filters.media_types = vec![MediaType::Video];
    r.text = "photo".into();

    // Act
    let by_month = h.section_index(&r, &utc).unwrap();
    r.sort.field = SortField::Name;
    let by_name = h.section_index(&r, &utc).unwrap();

    // Assert: 필터·질의가 총합에 반영된다 (영상 20건, 이름 "a-..." 만)
    assert_eq!(by_month.total, 20);
    assert_eq!(by_month.sections.iter().map(|s| s.count).sum::<u64>(), 20);
    assert_eq!(pairs(&by_name), [("a".to_string(), 20)]);
    // 총합은 같은 조건의 search() total 과 같다
    let page = h.search(&r).unwrap();
    assert_eq!(page.total, by_name.total);
    // limit/cursor/at 는 무시된다
    r.limit = 0;
    r.at = Some(99);
    r.cursor = Some("garbage".into());
    assert_eq!(h.section_index(&r, &utc).unwrap().total, 20);
}

#[test]
fn t_sect_15_sorts_without_sections_return_one_bucket_and_empty_results_return_none() {
    // Arrange
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &(1..=12).map(|i| doc(i, "x.jpg")).collect::<Vec<_>>());
    let h = SearchHandle::for_index(&idx);

    for field in [SortField::Size, SortField::Duration, SortField::Relevance] {
        // Act
        let s = h.section_index(&creq(field, true), &utc).unwrap();
        // Assert
        assert_eq!(s.kind, SectionKind::None, "{field:?}");
        assert_eq!(pairs(&s), [("all".to_string(), 12)]);
        assert_eq!(s.total, 12);
    }
    // 결과 없음: 구간 없음, total 0 (월·이름 정렬 모두)
    for field in [SortField::CapturedAt, SortField::Name, SortField::Size] {
        let mut r = creq(field, false);
        r.filters.media_types = vec![MediaType::Audio];
        let s = h.section_index(&r, &utc).unwrap();
        assert_eq!((s.total, s.sections.len()), (0, 0), "{field:?}");
    }
    // 촬영일이 하나도 없는 인덱스: 전부 none
    let s = h.section_index(&creq(SortField::CapturedAt, false), &utc).unwrap();
    assert_eq!(pairs(&s), [("none".to_string(), 12)]);
}

#[test]
fn t_sect_16_title_sort_buckets_by_title_then_file_name() {
    // Arrange: 제목이 있으면 제목, 없으면 파일명이 정렬·구간 기준
    let mut a = doc(1, "alpha.jpg");
    a.title = "Zeta".into();
    let b = doc(2, "beta.jpg");
    let dir = tempdir().unwrap();
    let idx = indexed(&dir, &[a, b]);
    let h = SearchHandle::for_index(&idx);

    // Act
    let by_title = h.section_index(&creq(SortField::Title, false), &utc).unwrap();
    let by_name = h.section_index(&creq(SortField::Name, false), &utc).unwrap();

    // Assert
    assert_eq!(pairs(&by_title), [("b".to_string(), 1), ("z".to_string(), 1)]);
    assert_eq!(pairs(&by_name), [("a".to_string(), 1), ("b".to_string(), 1)]);
}

#[test]
fn t_sect_17_section_index_before_the_index_is_ready_fails() {
    // Arrange: 인덱스를 게시하지 않은 핸들
    let h = SearchHandle::new();
    // Act + Assert
    assert!(matches!(
        h.section_index(&creq(SortField::Name, false), &utc),
        Err(mam_search::SearchError::NotReady)
    ));
}
