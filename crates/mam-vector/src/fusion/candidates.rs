//! D3·D4: 벡터 후보 수집 — 필터를 순위 계산 전에 적용하고 자산 단위로 묶는다.

use super::plan::{CandidatePlan, FilterStrategy, choose_filter_strategy};
use crate::error::Result;
use crate::flat::FlatIndex;
use crate::group::{AssetHit, group_by_asset};
use crate::idset::IdSet;
use crate::types::{AnnIndex, EmbeddingId, Hit, MAX_K, SearchParams};

/// 자산 단위 벡터 후보를 최대 `plan.per_source` 개 (유사도 내림차순) 돌려준다.
///
/// - `allowed = None` : 필터 없음
/// - 허용 비율 < [`super::SUBSET_SELECTIVITY`] : 허용 집합만 전수 비교 (`Subset`)
/// - 그 이상 : 전체 스캔 + 스캔 중 predicate (`FullScanPredicate`, 비트맵 판정)
///
/// 두 경로 모두 상위 k 를 뽑은 뒤 거르지 않으므로 허용 항목이 후보에서 밀려 누락되지 않는다.
/// 한 자산의 프레임이 상위를 독차지해 자산 수가 모자라면 프레임 `k` 를 두 배씩 키워 다시 찾는다.
/// `total` 은 전체 벡터 수 (호출자 제공 — 전략 선택용).
pub fn vector_candidates(
    index: &FlatIndex,
    q: &[f32],
    plan: &CandidatePlan,
    allowed: Option<&[EmbeddingId]>,
    total: usize,
    asset_of: &dyn Fn(EmbeddingId) -> Option<u64>,
) -> Result<Vec<AssetHit>> {
    vector_candidates_with_set(index, q, plan, allowed, None, total, asset_of)
}

/// [`vector_candidates`] + 이미 만들어 둔 `allowed` 의 집합(`set`) 재사용. 활성 맵 캐시가 가진 존재 비트맵을
/// 그대로 넘겨 질의마다 220만 id 로 집합을 다시 만들지 않게 한다. `set` 은 `allowed` 와 같은 id 들이어야 한다.
pub fn vector_candidates_with_set(
    index: &FlatIndex,
    q: &[f32],
    plan: &CandidatePlan,
    allowed: Option<&[EmbeddingId]>,
    set: Option<&IdSet>,
    total: usize,
    asset_of: &dyn Fn(EmbeddingId) -> Option<u64>,
) -> Result<Vec<AssetHit>> {
    let want = plan.per_source.max(1);
    let cap = MAX_K.min(index.len().max(1));
    let strategy = allowed.map(|a| choose_filter_strategy(a.len(), total));
    let built: Option<IdSet> = match (strategy, set) {
        (Some(FilterStrategy::FullScanPredicate), None) => allowed.map(IdSet::from_ids),
        _ => None,
    };
    let set = set.or(built.as_ref());
    let pred = |id: EmbeddingId| set.is_none_or(|s| s.contains(id));
    let params = SearchParams::default();
    let mut k = want.min(cap);
    loop {
        let hits: Vec<Hit> = match (strategy, allowed) {
            (Some(FilterStrategy::Subset), Some(a)) => index.search_subset(q, k, a)?,
            (Some(FilterStrategy::FullScanPredicate), _) => {
                index.search(q, k, &params, Some(&pred))?
            }
            _ => index.search(q, k, &params, None)?,
        };
        // 요청한 k 보다 적게 돌아왔으면 더 키워도 새로 나올 것이 없다
        let exhausted = hits.len() < k;
        let mut grouped = group_by_asset(&hits, asset_of);
        if grouped.len() >= want || exhausted || k >= cap {
            grouped.truncate(want);
            return Ok(grouped);
        }
        k = (k * 2).min(cap);
    }
}
