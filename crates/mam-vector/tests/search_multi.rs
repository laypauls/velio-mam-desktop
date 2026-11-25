//! T-perf-01·02: `FlatIndex::search_multi` (다중 질의 단일 순회) == 질의별 `search` 의 벡터별 최대 (점수는 비트까지 같다).
//! 분류: Happy / Boundary / Validation.

mod common;

use std::collections::HashMap;

use common::Rng;
use mam_vector::{AnnIndex, FlatIndex, Quantization, SearchParams, VectorError, normalized};

fn index(dim: usize, quant: Quantization, n: usize, seed: u64) -> FlatIndex {
    let mut rng = Rng(seed);
    let idx = FlatIndex::new(dim, quant);
    for i in 0..n {
        let v: Vec<f32> = (0..dim).map(|_| rng.signed()).collect();
        idx.add(i as u64 * 3 + 1, &normalized(&v)).unwrap();
    }
    idx
}

fn queries(dim: usize, n: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|_| (0..dim).map(|_| rng.signed()).collect())
        .collect()
}

// ---------------------------------------------------------------- T-perf-01 (Happy)
#[test]
fn t_perf_01_multi_scores_equal_per_query_max() {
    common::affinity::exclude_inconsistent_cores();
    for (dim, quant) in [
        (32, Quantization::F32),
        (48, Quantization::F16),
        (512, Quantization::F16),
        (32, Quantization::I8),
    ] {
        // Arrange
        let n = 3_000;
        let idx = index(dim, quant, n, 5);
        let qs = queries(dim, 5, 9);
        let p = SearchParams::default();
        // 정답: 질의마다 전체 순위 -> 벡터별 최대 점수
        let mut want: HashMap<u64, f32> = HashMap::new();
        for q in &qs {
            for h in idx.search(q, n.min(10_000), &p, None).unwrap() {
                want.entry(h.id)
                    .and_modify(|b| *b = b.max(h.score))
                    .or_insert(h.score);
            }
        }
        let mut ranked: Vec<(u64, f32)> = want.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        // Act
        let got = idx.search_multi(&qs, 200, None).unwrap();
        // Assert: 반환 id 마다 점수가 정답 최대값과 같고(비트까지 같다 — 커널 누적 순서 고정),
        // 점수는 내림차순이며, 200위 경계 밖의 더 높은 벡터를 놓치지 않았다
        assert_eq!(got.len(), 200);
        // 순서(동점은 id 오름차순)까지 정답 순위의 앞 200 개와 같다
        assert_eq!(
            got.iter().map(|h| h.id).collect::<Vec<_>>(),
            ranked.iter().take(200).map(|r| r.0).collect::<Vec<_>>(),
            "{dim} {quant:?} order"
        );
        let truth: HashMap<u64, f32> = ranked.iter().copied().collect();
        for g in &got {
            assert!(g.score == truth[&g.id], "{dim} {quant:?} id {}", g.id);
        }
        assert!(got.windows(2).all(|w| w[0].score >= w[1].score));
        let cutoff = got.last().unwrap().score;
        let returned: std::collections::HashSet<u64> = got.iter().map(|h| h.id).collect();
        for (id, s) in &ranked {
            assert!(
                returned.contains(id) || *s <= cutoff,
                "{dim} {quant:?} missed {id}"
            );
        }
    }
}

#[test]
fn t_perf_01_filter_applies_inside_the_scan() {
    common::affinity::exclude_inconsistent_cores();
    let idx = index(32, Quantization::F16, 2_000, 3);
    let qs = queries(32, 3, 4);
    let even = |id: u64| id.is_multiple_of(2);
    let got = idx.search_multi(&qs, 50, Some(&even)).unwrap();
    assert_eq!(got.len(), 50);
    assert!(got.iter().all(|h| h.id.is_multiple_of(2)));
}

// ---------------------------------------------------------------- T-perf-02 (Boundary / Validation)
#[test]
fn t_perf_02_boundaries_and_validation() {
    common::affinity::exclude_inconsistent_cores();
    let idx = index(32, Quantization::F32, 10, 1);
    let qs = queries(32, 2, 2);
    // k 가 크면 있는 만큼
    assert_eq!(idx.search_multi(&qs, 100, None).unwrap().len(), 10);
    // 질의 하나는 search 와 같다
    let one = idx.search_multi(&qs[..1], 5, None).unwrap();
    let base = idx
        .search(&qs[0], 5, &SearchParams::default(), None)
        .unwrap();
    assert_eq!(one.len(), base.len());
    for (a, b) in one.iter().zip(&base) {
        assert_eq!(a.id, b.id);
        assert_eq!(a.score.to_bits(), b.score.to_bits());
    }
    // 잘못된 k·차원·영벡터
    assert!(matches!(
        idx.search_multi(&qs, 0, None),
        Err(VectorError::InvalidInput(_))
    ));
    let bad = vec![vec![1.0f32; 31]];
    assert!(matches!(
        idx.search_multi(&bad, 5, None),
        Err(VectorError::DimensionMismatch { .. })
    ));
    let zero = vec![vec![0.0f32; 32]];
    assert!(matches!(
        idx.search_multi(&zero, 5, None),
        Err(VectorError::InvalidInput(_))
    ));
}
