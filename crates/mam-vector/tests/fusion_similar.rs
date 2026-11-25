//! T-fus-07·08·09: 유사 미디어 질의. 분류: Happy / Business rule / Concurrency(cursor) / Validation.

mod common;

use std::collections::HashSet;

use common::{DIM, dataset};
use mam_vector::{
    AnnIndex, FlatIndex, Quantization, SimilarCursor, SimilarQuery, VectorError, find_similar,
};

const ASSETS: u64 = 60;

/// 자산 a 의 프레임 f 는 임베딩 id = a * 10 + f. 프레임 수는 자산마다 1~3.
fn frames_of(a: u64) -> u64 {
    a % 3 + 1
}

fn asset_of(e: u64) -> Option<u64> {
    Some(e / 10)
}

struct Fixture {
    idx: FlatIndex,
    vecs: Vec<(u64, Vec<f32>)>, // (asset, vector)
}

fn fixture() -> Fixture {
    let all = dataset(ASSETS as usize * 3, DIM, 11);
    let idx = FlatIndex::new(DIM, Quantization::F32);
    let mut vecs = Vec::new();
    let mut n = 0;
    for a in 0..ASSETS {
        for f in 0..frames_of(a) {
            idx.add(a * 10 + f, &all[n]).unwrap();
            vecs.push((a, all[n].clone()));
            n += 1;
        }
    }
    Fixture { idx, vecs }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn query_frames(fx: &Fixture, asset: u64) -> Vec<Vec<f32>> {
    fx.vecs
        .iter()
        .filter(|(a, _)| *a == asset)
        .map(|(_, v)| v.clone())
        .collect()
}

/// 정답: 자산별 (질의 프레임 x 자산 프레임) 최대 유사도(0..1 클램프), (점수 내림, id 오름).
fn brute(fx: &Fixture, qf: &[Vec<f32>], exclude: &[u64]) -> Vec<(u64, f32)> {
    let mut v: Vec<(u64, f32)> = (0..ASSETS)
        .filter(|a| !exclude.contains(a))
        .map(|a| {
            let best = fx
                .vecs
                .iter()
                .filter(|(x, _)| *x == a)
                .flat_map(|(_, v)| qf.iter().map(move |q| dot(q, v)))
                .fold(f32::NEG_INFINITY, f32::max);
            (a, best.clamp(0.0, 1.0))
        })
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

fn query<'a>(
    frames: &'a [Vec<f32>],
    exclude: &'a [u64],
    limit: usize,
    cursor: Option<SimilarCursor>,
) -> SimilarQuery<'a> {
    SimilarQuery {
        frames,
        exclude,
        limit,
        cursor,
        min_score: 0.0,
    }
}

fn assert_matches(got: &[(u64, f32)], want: &[(u64, f32)]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g.0, w.0);
        assert!((g.1 - w.1).abs() < 1e-4, "{g:?} vs {w:?}");
    }
}

// ---------------------------------------------------------------- T-fus-07 (Happy)
#[test]
fn t_fus_07_multi_frame_assets_return_once_with_max_similarity() {
    // Arrange: 프레임 3개인 기준 자산 2 (2 % 3 + 1 = 3)
    let fx = fixture();
    let qf = query_frames(&fx, 2);
    assert_eq!(qf.len(), 3);
    // Act
    let page = find_similar(&fx.idx, &query(&qf, &[2], 100, None), &asset_of).unwrap();
    // Assert: 자산당 1건, 점수는 프레임 쌍 최대 (0..1)
    assert_matches(&page.items, &brute(&fx, &qf, &[2]));
    let uniq: HashSet<u64> = page.items.iter().map(|i| i.0).collect();
    assert_eq!(uniq.len(), page.items.len());
    assert!(page.items.iter().all(|i| (0.0..=1.0).contains(&i.1)));
    assert!(page.next.is_none());
}

// ---------------------------------------------------------------- T-fus-08 (Business rule)
#[test]
fn t_fus_08_excludes_self_and_duplicates() {
    // Arrange: 기준 자산 5, 같은 해시 중복본으로 가정한 자산 6·7
    let fx = fixture();
    let qf = query_frames(&fx, 5);
    let exclude = [5, 6, 7];
    // Act
    let page = find_similar(&fx.idx, &query(&qf, &exclude, 10, None), &asset_of).unwrap();
    // Assert: 제외 자산이 없고 정답과 같다. 제외 때문에 빈 자리가 생기지 않는다 (10건)
    assert_eq!(page.items.len(), 10);
    assert!(page.items.iter().all(|i| !exclude.contains(&i.0)));
    assert_matches(&page.items, &brute(&fx, &qf, &exclude)[..10]);
}

// ---------------------------------------------------------------- T-fus-09 (Concurrency: cursor)
#[test]
fn t_fus_09_cursor_pages_have_no_duplicates_or_gaps() {
    // Arrange
    let fx = fixture();
    let qf = query_frames(&fx, 4);
    let want = brute(&fx, &qf, &[4]);
    // Act: 7건씩 cursor 로 끝까지 (문자열 왕복 포함)
    let mut got: Vec<(u64, f32)> = Vec::new();
    let mut cursor: Option<SimilarCursor> = None;
    let mut pages = 0;
    loop {
        let page = find_similar(&fx.idx, &query(&qf, &[4], 7, cursor), &asset_of).unwrap();
        got.extend(&page.items);
        pages += 1;
        match page.next {
            Some(c) => cursor = Some(SimilarCursor::decode(&c.encode()).unwrap()),
            None => break,
        }
        assert!(pages < 50, "cursor must terminate");
    }
    // Assert
    assert_eq!(pages, want.len().div_ceil(7));
    assert_matches(&got, &want);
    let uniq: HashSet<u64> = got.iter().map(|i| i.0).collect();
    assert_eq!(uniq.len(), got.len());
}

#[test]
fn t_fus_09_cursor_encoding_roundtrips_and_rejects_malformed() {
    let fx = fixture();
    let qf = query_frames(&fx, 1);
    let page = find_similar(&fx.idx, &query(&qf, &[1], 3, None), &asset_of).unwrap();
    let c = page.next.unwrap();
    assert_eq!(SimilarCursor::decode(&c.encode()).unwrap(), c);
    for bad in [
        "",
        "garbage",
        "s1.zz.1",
        "s1.3f800000",
        "s1.3f800000.1.extra",
        "s2.3f800000.1",
        "s1.40000000.1",
        "s1.3f800000.xyz",
    ] {
        assert!(
            matches!(
                SimilarCursor::decode(bad),
                Err(VectorError::InvalidInput(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn t_fus_validation_rejects_empty_frames_and_bad_limit() {
    let fx = fixture();
    let qf = query_frames(&fx, 1);
    let none: Vec<Vec<f32>> = vec![];
    for (frames, limit) in [(&none, 5), (&qf, 0), (&qf, 1001)] {
        let r = find_similar(&fx.idx, &query(frames, &[], limit, None), &asset_of);
        assert!(matches!(r, Err(VectorError::InvalidInput(_))));
    }
    // 차원이 다른 프레임은 인덱스 오류로 전달된다
    let bad = vec![vec![1.0f32; DIM + 1]];
    let r = find_similar(&fx.idx, &query(&bad, &[], 5, None), &asset_of);
    assert!(matches!(r, Err(VectorError::DimensionMismatch { .. })));
}

// ---------------------------------------------------------------- T-fus-10 (Boundary: min_score)
#[test]
fn t_fus_10_min_score_stops_at_the_threshold_and_ends_paging() {
    // Arrange: 제한 없는 정답에서 중간 항목의 점수를 기준으로 삼는다
    let fx = fixture();
    let qf = query_frames(&fx, 2);
    let all = find_similar(&fx.idx, &query(&qf, &[2], 100, None), &asset_of).unwrap().items;
    assert!(all.len() > 6);
    let min = all[5].1;
    let want: Vec<(u64, f32)> = all.iter().copied().filter(|i| i.1 >= min).collect();

    // Act: 한 번에 / 페이지 끝이 기준과 정확히 맞는 limit / 기준보다 큰 limit
    let q = |limit, cursor, min_score| SimilarQuery {
        min_score,
        ..query(&qf, &[2], limit, cursor)
    };
    let whole = find_similar(&fx.idx, &q(100, None, min), &asset_of).unwrap();
    let exact = find_similar(&fx.idx, &q(want.len(), None, min), &asset_of).unwrap();
    let first = find_similar(&fx.idx, &q(2, None, min), &asset_of).unwrap();
    let off = find_similar(&fx.idx, &q(100, None, 0.0), &asset_of).unwrap();

    // Assert: 같음 포함, 미만 제외, 기준 도달 시 next 없음, 0 은 제한 없음
    assert_eq!(whole.items, want);
    assert!(whole.next.is_none());
    assert_eq!(exact.items, want);
    assert!(exact.next.is_none());
    assert_eq!(first.items.len(), 2);
    assert!(first.next.is_some());
    assert_eq!(off.items, all);
    // 검증: 범위 밖 min_score
    for bad in [-0.1, 1.1, f32::NAN] {
        assert!(find_similar(&fx.idx, &q(5, None, bad), &asset_of).is_err());
    }
}
