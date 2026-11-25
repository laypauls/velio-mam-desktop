//! T-perf-03: 단일 순회 `find_similar` == 이전 알고리즘(프레임별 필터 스캔) — 무작위 데이터 대조.
//! 분류: Happy / Business rule(제외·폐기 벡터) / Boundary(limit 1, 전체 이상, 제외 다수).
//! 같은 커널·고정 누적 순서이므로 같은 자산 집합 + 점수 비트 동일로 비교한다.

mod common;
#[path = "../examples/common/legacy_similar.rs"]
mod legacy_similar;

use common::{Rng, dataset};
use legacy_similar::legacy_find_similar;
use mam_vector::{AnnIndex, FlatIndex, Quantization, SimilarQuery, find_similar};

const DIM: usize = 32;
const ASSETS: u64 = 300;

/// 자산 a 는 프레임 1~5개. 임베딩 id = a * 10 + f. 자산 % 7 == 0 의 첫 프레임은 폐기(맵에 없음).
fn frames_of(a: u64) -> u64 {
    a % 5 + 1
}

fn asset_of(e: u64) -> Option<u64> {
    let (a, f) = (e / 10, e % 10);
    (!(a % 7 == 0 && f == 0)).then_some(a)
}

fn build(quant: Quantization) -> (FlatIndex, Vec<Vec<f32>>) {
    let all = dataset(ASSETS as usize * 5, DIM, 21);
    let idx = FlatIndex::new(DIM, quant);
    let mut n = 0;
    for a in 1..=ASSETS {
        for f in 0..frames_of(a) {
            idx.add(a * 10 + f, &all[n]).unwrap();
            n += 1;
        }
    }
    (idx, all)
}

fn assert_equivalent(got: &[(u64, f32)], want: &[(u64, f32)], ctx: &str) {
    assert_eq!(got.len(), want.len(), "{ctx}: length");
    let mut g: Vec<u64> = got.iter().map(|x| x.0).collect();
    let mut w: Vec<u64> = want.iter().map(|x| x.0).collect();
    g.sort_unstable();
    w.sort_unstable();
    assert_eq!(g, w, "{ctx}: asset set");
    for (a, b) in got.iter().zip(want) {
        assert_eq!(a.1.to_bits(), b.1.to_bits(), "{ctx}: {a:?} vs {b:?}");
    }
    assert!(got.windows(2).all(|p| p[0].1 >= p[1].1), "{ctx}: order");
}

#[test]
fn t_perf_03_new_find_similar_matches_legacy_algorithm_on_random_queries() {
    common::affinity::exclude_inconsistent_cores();
    for quant in [Quantization::F32, Quantization::F16] {
        // Arrange
        let (idx, _) = build(quant);
        let mut rng = Rng(5);
        for case in 0..40 {
            let nq = 1 + (rng.next_u64() % 5) as usize;
            let frames: Vec<Vec<f32>> = (0..nq)
                .map(|_| (0..DIM).map(|_| rng.signed()).collect())
                .collect();
            let limit = [1usize, 5, 20, 100, 400][(rng.next_u64() % 5) as usize];
            let ex_n = (rng.next_u64() % 4) as usize * 3; // 0, 3, 6, 9 개 제외 (중복 포함 가능)
            let exclude: Vec<u64> = (0..ex_n).map(|_| 1 + rng.next_u64() % ASSETS).collect();
            // Act
            let want = legacy_find_similar(&idx, &frames, &exclude, limit, &asset_of).unwrap();
            let got = find_similar(
                &idx,
                &SimilarQuery {
                    frames: &frames,
                    exclude: &exclude,
                    limit,
                    cursor: None,
                    min_score: 0.0,
                },
                &asset_of,
            )
            .unwrap();
            // Assert
            let ctx = format!("{quant:?} case {case} frames {nq} limit {limit} exclude {ex_n}");
            assert_equivalent(&got.items, &want, &ctx);
            assert!(got.items.iter().all(|(a, _)| !exclude.contains(a)), "{ctx}");
            assert!(
                got.items
                    .iter()
                    .all(|(a, _)| asset_of(a * 10 + 1).is_some())
            );
        }
    }
}

#[test]
fn t_perf_03_many_excluded_duplicates_do_not_starve_the_result() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 질의 자산의 사본 200개를 제외 (실제 앱은 최대 1,000개) — 상위가 전부 제외 대상이어도 limit 개를 채운다
    let (idx, all) = build(Quantization::F32);
    let frames = vec![all[0].clone(), all[1].clone()];
    let exclude: Vec<u64> = (1..=200).collect();
    // Act
    let got = find_similar(
        &idx,
        &SimilarQuery {
            frames: &frames,
            exclude: &exclude,
            limit: 50,
            cursor: None,
            min_score: 0.0,
        },
        &asset_of,
    )
    .unwrap();
    let want = legacy_find_similar(&idx, &frames, &exclude, 50, &asset_of).unwrap();
    // Assert
    assert_eq!(got.items.len(), 50);
    assert_equivalent(&got.items, &want, "excluded duplicates");
    assert!(got.items.iter().all(|(a, _)| *a > 200));
}
