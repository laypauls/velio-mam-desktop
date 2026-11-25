//! D1·D6: Reciprocal Rank Fusion. `score(a) = Σ w_s / (k + rank_s(a))`, 순위는 1 부터.

use std::collections::HashMap;

use crate::group::AssetHit;

/// RRF 파라미터. 기본값은 문헌 기본(`k = 60`, 가중치 1.0) — 실제 데이터 평가 전의 잠정값이다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RrfParams {
    pub k: f32,
    pub keyword_weight: f32,
    pub vector_weight: f32,
}

impl Default for RrfParams {
    fn default() -> Self {
        Self {
            k: 60.0,
            keyword_weight: 1.0,
            vector_weight: 1.0,
        }
    }
}

/// 결합 결과 한 건. UI 에는 순서만 쓰고 점수는 디버깅용이다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FusedHit {
    pub asset_id: u64,
    pub rrf: f64,
    /// 키워드 목록에서의 순위 (1 부터, 중복 제거 후). 키워드 후보가 아니면 `None`.
    pub keyword_rank: Option<usize>,
    pub vector_rank: Option<usize>,
    /// 벡터 후보일 때 자산의 최대 코사인 유사도 (표시·디버깅용 — 순서에는 영향 없음).
    pub similarity: Option<f32>,
}

/// 음수·NaN 은 0 으로 취급한다 (순위 기여를 없애거나 `k + rank` 가 0 이하가 되는 것을 막는다).
fn sane(x: f32) -> f64 {
    if x.is_finite() && x > 0.0 {
        x as f64
    } else {
        0.0
    }
}

/// 키워드(자산 id, 순위순)와 벡터(자산 단위, 유사도 내림차순) 목록을 RRF 로 결합한다.
///
/// - 같은 id 가 한 목록에 반복되면 **첫 등장만** 쓴다 (순위는 중복 제거 후 위치)
/// - 한쪽에만 있는 후보도 포함한다
/// - 동점은 키워드 순위(없으면 뒤) -> 벡터 순위(없으면 뒤) -> `asset_id` 로 결정적으로 정렬한다
pub fn fuse_rrf(keyword: &[u64], vector: &[AssetHit], params: &RrfParams) -> Vec<FusedHit> {
    let k = sane(params.k);
    let (wk, wv) = (sane(params.keyword_weight), sane(params.vector_weight));
    let mut map: HashMap<u64, FusedHit> = HashMap::with_capacity(keyword.len() + vector.len());

    let mut rank = 0usize;
    for &id in keyword {
        if map.get(&id).is_some_and(|h| h.keyword_rank.is_some()) {
            continue;
        }
        rank += 1;
        map.entry(id)
            .or_insert(FusedHit {
                asset_id: id,
                rrf: 0.0,
                keyword_rank: None,
                vector_rank: None,
                similarity: None,
            })
            .keyword_rank = Some(rank);
    }
    let mut rank = 0usize;
    for v in vector {
        if map
            .get(&v.asset_id)
            .is_some_and(|h| h.vector_rank.is_some())
        {
            continue;
        }
        rank += 1;
        let h = map.entry(v.asset_id).or_insert(FusedHit {
            asset_id: v.asset_id,
            rrf: 0.0,
            keyword_rank: None,
            vector_rank: None,
            similarity: None,
        });
        h.vector_rank = Some(rank);
        h.similarity = Some(v.score);
    }

    let mut out: Vec<FusedHit> = map
        .into_values()
        .map(|mut h| {
            h.rrf = h.keyword_rank.map_or(0.0, |r| wk / (k + r as f64))
                + h.vector_rank.map_or(0.0, |r| wv / (k + r as f64));
            h
        })
        .collect();
    out.sort_by(|a, b| {
        b.rrf
            .total_cmp(&a.rrf)
            .then_with(|| {
                a.keyword_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&b.keyword_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| {
                a.vector_rank
                    .unwrap_or(usize::MAX)
                    .cmp(&b.vector_rank.unwrap_or(usize::MAX))
            })
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });
    out
}

/// [`fuse_hybrid`] 결과.
#[derive(Clone, Debug, PartialEq)]
pub struct HybridOutcome {
    pub hits: Vec<FusedHit>,
    /// 벡터 후보가 없어(모델/인덱스 미준비) 키워드 순서만으로 만든 결과 -> `SearchPage.degraded`.
    pub degraded: bool,
}

/// 하이브리드 진입점. `vector = None`(모델·인덱스 사용 불가)이면 키워드 순서 그대로 + `degraded = true`.
/// `Some(&[])`(벡터 검색은 됐으나 후보 0)는 정상 결합이다 (`degraded = false`).
pub fn fuse_hybrid(
    keyword: &[u64],
    vector: Option<&[AssetHit]>,
    params: &RrfParams,
) -> HybridOutcome {
    HybridOutcome {
        hits: fuse_rrf(keyword, vector.unwrap_or(&[]), params),
        degraded: vector.is_none(),
    }
}
