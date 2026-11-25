//! D2·D3: 후보 수 계획(기본 400, page_end×4, 상한 2,000)과 필터 전략 선택.

/// 키워드·벡터 각 소스의 최소 후보 수.
pub const MIN_PER_SOURCE: usize = 400;
/// 소스당 후보 수 상한.
pub const MAX_PER_SOURCE: usize = 2_000;

/// 소스(키워드·벡터)별로 가져올 후보 수.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidatePlan {
    pub per_source: usize,
}

impl CandidatePlan {
    /// 상한에 닿아 더 키울 수 없다.
    pub fn is_capped(&self) -> bool {
        self.per_source >= MAX_PER_SOURCE
    }

    /// 후보를 두 배로 늘린 다음 계획 (상한 적용).
    pub fn grow(&self) -> Self {
        Self {
            per_source: (self.per_source * 2).min(MAX_PER_SOURCE),
        }
    }
}

/// `page_end` = 요청한 페이지의 끝 위치 (offset + limit). 후보 = `max(400, page_end × 4)`, 상한 2,000.
pub fn plan_candidates(page_end: usize) -> CandidatePlan {
    CandidatePlan {
        per_source: page_end
            .saturating_mul(4)
            .clamp(MIN_PER_SOURCE, MAX_PER_SOURCE),
    }
}

/// 결합 결과가 `page_end` 보다 적고, 어느 한쪽 후보가 꽉 찼으며(더 있을 수 있음), 아직 상한 전이면 `true`.
pub fn needs_more(
    fused_len: usize,
    page_end: usize,
    keyword_len: usize,
    vector_len: usize,
    plan: &CandidatePlan,
) -> bool {
    fused_len < page_end
        && !plan.is_capped()
        && (keyword_len >= plan.per_source || vector_len >= plan.per_source)
}

/// 허용 비율이 이 값 미만이면 허용 집합만 훑는다 (2%).
///
/// 근거 (P4 후속 측정, 벡터 44만 x 512-d f16, 12 스레드, 임베딩 단위 k=400, 집합 구성 포함 P50): 허용 1% Subset 1.0ms <
/// FullScan 1.5ms 지만 2% 2.8 vs 1.7, 5% 6.8 vs 2.5, 10% 5.7 vs 3.5, 33% 14.1 vs 8.9, 50% 19.5 vs 12.1, 70% 25.7 vs 15.2 로
/// 비트맵 판정 FullScan 이 앞선다 (이전 10% 임계는 해시 집합 판정 기준이었다). 근거 수치·조건은 `docs/requests/2026-10/`.
pub const SUBSET_SELECTIVITY: f64 = 0.02;

/// 벡터 후보를 구하는 필터 적용 방식. 두 경로 모두 **순위를 매기기 전에** 필터를 적용한다
/// (상위 k 를 뽑은 뒤 거르지 않는다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterStrategy {
    /// 허용 집합만 전수 비교 (`FlatIndex::search_subset`).
    Subset,
    /// 전체를 훑되 스캔 중 허용 판정(predicate) 을 적용한다 (`AnnIndex::search(.., filter)`).
    FullScanPredicate,
}

/// 허용 수/전체 수 < [`SUBSET_SELECTIVITY`] (2%) 이면 `Subset`, 그 이상이면 `FullScanPredicate`.
pub fn choose_filter_strategy(allowed: usize, total: usize) -> FilterStrategy {
    if (allowed as f64) < SUBSET_SELECTIVITY * total as f64 {
        FilterStrategy::Subset
    } else {
        FilterStrategy::FullScanPredicate
    }
}
