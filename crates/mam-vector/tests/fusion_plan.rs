//! T-fus-04·06: 후보 수 계획, needs_more, 필터 전략 선택. 분류: Business rule / Boundary.

use mam_vector::fusion::{MAX_PER_SOURCE, MIN_PER_SOURCE, SUBSET_SELECTIVITY};
use mam_vector::{FilterStrategy, choose_filter_strategy, needs_more, plan_candidates};

// ---------------------------------------------------------------- T-fus-04 (Business rule)
#[test]
fn t_fus_04_candidates_grow_with_page_depth_and_cap_at_2000() {
    assert_eq!(MIN_PER_SOURCE, 400);
    assert_eq!(plan_candidates(0).per_source, 400);
    assert_eq!(plan_candidates(50).per_source, 400);
    assert_eq!(plan_candidates(100).per_source, 400); // 경계: 100 x 4 = 400
    assert_eq!(plan_candidates(101).per_source, 404);
    assert_eq!(plan_candidates(300).per_source, 1200);
    assert_eq!(plan_candidates(500).per_source, 2000);
    assert_eq!(plan_candidates(100_000).per_source, MAX_PER_SOURCE);
    assert_eq!(plan_candidates(usize::MAX).per_source, MAX_PER_SOURCE);
}

#[test]
fn t_fus_04_grow_doubles_until_cap() {
    let mut p = plan_candidates(50);
    let mut seen = vec![p.per_source];
    while !p.is_capped() {
        p = p.grow();
        seen.push(p.per_source);
    }
    assert_eq!(seen, vec![400, 800, 1600, 2000]);
    assert_eq!(p.grow(), p);
}

// ---------------------------------------------------------------- T-fus-06 (Boundary)
#[test]
fn t_fus_06_needs_more_when_short_and_a_source_is_full() {
    let plan = plan_candidates(50); // 400
    // 결과 부족 + 키워드 꽉 참 -> 확대 요청
    assert!(needs_more(30, 50, 400, 10, &plan));
    // 벡터가 꽉 찬 경우도 동일
    assert!(needs_more(30, 50, 10, 400, &plan));
    // 결과 부족해도 어느 쪽도 꽉 차지 않았으면 소진 -> 확대 불필요
    assert!(!needs_more(30, 50, 399, 399, &plan));
    // 결과가 충분하면 꽉 차 있어도 불필요 (경계: fused == page_end)
    assert!(!needs_more(50, 50, 400, 400, &plan));
    // 상한에 닿았으면 더 키울 수 없다
    let capped = plan_candidates(1000);
    assert!(!needs_more(10, 1000, 2000, 2000, &capped));
}

// ---------------------------------------------------------------- strategy (Business rule / Boundary)
#[test]
fn t_fus_strategy_uses_subset_below_two_percent() {
    assert_eq!(SUBSET_SELECTIVITY, 0.02);
    assert_eq!(choose_filter_strategy(1, 1000), FilterStrategy::Subset);
    assert_eq!(choose_filter_strategy(19, 1000), FilterStrategy::Subset);
    // 경계: 정확히 2% 부터 전체 스캔
    assert_eq!(
        choose_filter_strategy(20, 1000),
        FilterStrategy::FullScanPredicate
    );
    assert_eq!(
        choose_filter_strategy(100, 1000),
        FilterStrategy::FullScanPredicate
    );
    assert_eq!(
        choose_filter_strategy(900, 1000),
        FilterStrategy::FullScanPredicate
    );
    // 경계: 허용 0 건, 전체 0 건
    assert_eq!(choose_filter_strategy(0, 1000), FilterStrategy::Subset);
    assert_eq!(
        choose_filter_strategy(0, 0),
        FilterStrategy::FullScanPredicate
    );
}
