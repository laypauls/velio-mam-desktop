//! T-fus-05: 필터 우선 벡터 후보 — 전수 비교 정답 대비 누락 없음. 분류: Business rule / Boundary.

mod common;

use common::Rng;
use mam_vector::{
    AnnIndex, CandidatePlan, FilterStrategy, FlatIndex, Quantization, SearchParams,
    choose_filter_strategy, exact_topk_f32, normalized, vector_candidates,
};

const DIM: usize = 64;
const N: usize = 20_000;

fn rows(n: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(n * DIM);
    for _ in 0..n {
        let v: Vec<f32> = (0..DIM).map(|_| rng.signed()).collect();
        out.extend(normalized(&v));
    }
    out
}

/// 임베딩 id = 1..=N, 자산 id = 임베딩 id (프레임 1개).
fn build() -> (FlatIndex, Vec<f32>) {
    let data = rows(N, 7);
    let idx = FlatIndex::new(DIM, Quantization::F32);
    for i in 0..N {
        idx.add(i as u64 + 1, &data[i * DIM..(i + 1) * DIM])
            .unwrap();
    }
    (idx, data)
}

/// 허용 집합에 대한 전수 비교 정답 (id 순서).
fn exact_over(data: &[f32], allowed: &[u64], q: &[f32], k: usize) -> Vec<u64> {
    let mut sub_rows = Vec::new();
    for &a in allowed {
        let i = (a - 1) as usize;
        sub_rows.extend_from_slice(&data[i * DIM..(i + 1) * DIM]);
    }
    exact_topk_f32(allowed, &sub_rows, DIM, q, k, 4)
        .into_iter()
        .map(|h| h.id)
        .collect()
}

fn allow_set(step: u64, offset: u64) -> Vec<u64> {
    (1..=N as u64).filter(|i| i % step == offset).collect()
}

fn check(allowed: &[u64], k: usize) {
    let (idx, data) = build();
    let q = rows(1, 99);
    let plan = CandidatePlan { per_source: k };
    let got = vector_candidates(&idx, &q, &plan, Some(allowed), N, &Some).unwrap();
    let want = exact_over(&data, allowed, &q, k);
    let got_ids: Vec<u64> = got.iter().map(|h| h.asset_id).collect();
    assert_eq!(got_ids, want, "filtered candidates must equal exact answer");
    assert!(got_ids.iter().all(|a| allowed.binary_search(a).is_ok()));
}

// ---------------------------------------------------------------- T-fus-05 (Business rule)
#[test]
fn t_fus_05_one_percent_allow_set_uses_subset_and_misses_nothing() {
    // Arrange: 허용 1% (200 건) — 선택도 < 2% -> Subset
    let allowed = allow_set(100, 3);
    assert_eq!(allowed.len(), 200);
    // Act + Assert
    check(&allowed, 50);
}

#[test]
fn t_fus_05_thirty_percent_allow_set_uses_full_scan_predicate_and_misses_nothing() {
    assert_eq!(
        choose_filter_strategy(allow_set(3, 1).len(), N),
        FilterStrategy::FullScanPredicate
    );
    // 허용 약 33% -> FullScanPredicate (상위 k 뒤 거르기가 아니라 스캔 중 판정)
    let allowed = allow_set(3, 1);
    check(&allowed, 100);
}

#[test]
fn t_fus_05_post_filtering_a_top_k_would_have_missed_results() {
    // 대조군: 필터 없이 상위 50 만 받아 거르면 1% 허용 집합에서 거의 남지 않는다
    let (idx, _) = build();
    let q = rows(1, 99);
    let allowed = allow_set(100, 3);
    let top = idx.search(&q, 50, &SearchParams::default(), None).unwrap();
    let survivors = top
        .iter()
        .filter(|h| allowed.binary_search(&h.id).is_ok())
        .count();
    assert!(survivors < 10, "survivors={survivors}");
}

#[test]
fn t_fus_05_boundary_empty_none_and_small_allow_sets() {
    let (idx, _) = build();
    let q = rows(1, 5);
    let plan = CandidatePlan { per_source: 20 };
    // 허용 집합이 비면 결과도 비어 있다
    let empty = vector_candidates(&idx, &q, &plan, Some(&[]), N, &Some).unwrap();
    assert!(empty.is_empty());
    // 필터 없음
    let none = vector_candidates(&idx, &q, &plan, None, N, &Some).unwrap();
    assert_eq!(none.len(), 20);
    // 허용 집합이 후보 수보다 작으면 있는 만큼만
    let few = vector_candidates(&idx, &q, &plan, Some(&[1, 2, 3]), N, &Some).unwrap();
    assert_eq!(few.len(), 3);
    // 인덱스에 없는 id 는 건너뛴다
    let ghost = vector_candidates(&idx, &q, &plan, Some(&[N as u64 + 5]), N, &Some).unwrap();
    assert!(ghost.is_empty());
}

#[test]
fn t_fus_05_multi_frame_assets_are_deduped_when_frames_dominate() {
    // Arrange: 자산 하나(1)의 프레임 30개가 질의와 거의 같고, 다른 자산 20개는 멀다
    let idx = FlatIndex::new(DIM, Quantization::F32);
    let base = rows(1, 1);
    let mut rng = Rng(3);
    for f in 0..30u64 {
        let v: Vec<f32> = base.iter().map(|x| x + 0.01 * rng.signed()).collect();
        idx.add(1000 + f, &v).unwrap();
    }
    let others = rows(20, 2);
    for i in 0..20usize {
        idx.add(i as u64 + 1, &others[i * DIM..(i + 1) * DIM])
            .unwrap();
    }
    let asset_of = |e: u64| Some(if e >= 1000 { 1 } else { e + 100 });
    let plan = CandidatePlan { per_source: 10 };
    // Act: 프레임 k=10 으로는 자산이 1개뿐 -> k 를 키워 자산 10개를 채운다
    let got = vector_candidates(&idx, &base, &plan, None, 50, &asset_of).unwrap();
    // Assert
    assert_eq!(got.len(), 10);
    assert_eq!(got[0].asset_id, 1);
    let mut uniq: Vec<u64> = got.iter().map(|h| h.asset_id).collect();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), 10);
}

// 임계(2%) 양쪽 경계와 높은 선택도에서도 두 경로 모두 정답과 같다 (전략이 바뀌어도 결과는 같아야 한다)
#[test]
fn t_fus_05_both_strategies_match_exact_answer_around_the_threshold_and_at_high_selectivity() {
    // 392 건(1.96%) -> Subset, 400 건(2.0%) -> FullScan, 50%·70% -> FullScan
    for (step, offset, want_subset) in [(51u64, 1u64, true), (50, 1, false)] {
        let allowed = allow_set(step, offset);
        let strategy = choose_filter_strategy(allowed.len(), N);
        assert_eq!(strategy == FilterStrategy::Subset, want_subset, "{step}");
        check(&allowed, 60);
    }
    let half = allow_set(2, 0);
    assert_eq!(
        choose_filter_strategy(half.len(), N),
        FilterStrategy::FullScanPredicate
    );
    check(&half, 100);
    let seventy: Vec<u64> = (1..=N as u64).filter(|i| i % 10 < 7).collect();
    assert_eq!(
        choose_filter_strategy(seventy.len(), N),
        FilterStrategy::FullScanPredicate
    );
    check(&seventy, 100);
}
