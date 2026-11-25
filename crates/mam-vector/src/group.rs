//! 다중 프레임 -> 자산 단위 결과 (D6) 와 후필터 + 후보 확대 (F2).
//!
//! 영상 1개가 여러 프레임 벡터를 가지므로 프레임 단위로 k 개를 찾은 뒤 `asset_id` 로 묶는다.
//! 자산 점수는 프레임 최대 점수다. 같은 자산의 프레임이 상위를 독차지하면 자산 수가 모자라므로 k 를 키워 다시 찾는다.

use std::collections::HashMap;

use crate::error::Result;
use crate::types::{AnnIndex, EmbeddingId, Filter, Hit, MAX_K, SearchParams};

/// 자산 단위 결과 한 건.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssetHit {
    pub asset_id: u64,
    /// 해당 자산 프레임들 중 최대 코사인 유사도.
    pub score: f32,
    /// 최대 점수를 낸 프레임의 `embedding_id`.
    pub embedding_id: EmbeddingId,
}

/// `asset_of` 가 `None` 을 돌려주는 벡터(고아 id)는 버린다. 결과는 점수 내림차순, `asset_id` 중복 없음.
pub fn group_by_asset(
    hits: &[Hit],
    asset_of: &dyn Fn(EmbeddingId) -> Option<u64>,
) -> Vec<AssetHit> {
    let mut best: HashMap<u64, AssetHit> = HashMap::new();
    for h in hits {
        let Some(asset_id) = asset_of(h.id) else {
            continue;
        };
        best.entry(asset_id)
            .and_modify(|a| {
                // 동점이면 embedding_id 가 작은 쪽 — 입력 순서와 무관하게 결정적이다
                if h.score > a.score || (h.score == a.score && h.id < a.embedding_id) {
                    a.score = h.score;
                    a.embedding_id = h.id;
                }
            })
            .or_insert(AssetHit {
                asset_id,
                score: h.score,
                embedding_id: h.id,
            });
    }
    let mut v: Vec<AssetHit> = best.into_values().collect();
    v.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.asset_id.cmp(&b.asset_id))
    });
    v
}

#[derive(Debug)]
pub struct AssetSearch {
    pub hits: Vec<AssetHit>,
    /// 마지막으로 요청한 프레임 단위 k — "자산 N 개를 채우는 데 필요한 k".
    pub k_used: usize,
    pub rounds: usize,
}

/// 자산 `want` 개를 채울 때까지 k 를 두 배로 키우며 찾는다 (최대 [`MAX_K`] 또는 인덱스 크기).
pub fn search_assets(
    index: &dyn AnnIndex,
    q: &[f32],
    want: usize,
    initial_k: usize,
    params: &SearchParams,
    filter: Option<Filter<'_>>,
    asset_of: &dyn Fn(EmbeddingId) -> Option<u64>,
) -> Result<AssetSearch> {
    let cap = MAX_K.min(index.len().max(1));
    let mut k = initial_k.clamp(1, cap);
    let mut rounds = 0;
    loop {
        rounds += 1;
        let hits = index.search(q, k, params, filter)?;
        let mut grouped = group_by_asset(&hits, asset_of);
        // 후보가 k 보다 적게 돌아왔다면 더 키워도 새로 나올 것이 없다 (필터 포함)
        let exhausted = hits.len() < k;
        if grouped.len() >= want || k >= cap || exhausted {
            grouped.truncate(want);
            return Ok(AssetSearch {
                hits: grouped,
                k_used: k,
                rounds,
            });
        }
        k = (k * 2).min(cap);
    }
}

#[derive(Debug)]
pub struct PostFiltered {
    pub hits: Vec<Hit>,
    /// 마지막으로 요청한 후보 수.
    pub k_final: usize,
    pub rounds: usize,
    /// 후보 상한/인덱스 끝에 닿았는데도 `k` 개를 못 채웠다 (선택도가 너무 낮다).
    pub incomplete: bool,
}

/// F2: 필터 없이 `k × factor` 후보를 받아 거른다. 모자라면 4배씩 키워 다시 찾는다.
pub fn search_post_filter(
    index: &dyn AnnIndex,
    q: &[f32],
    k: usize,
    params: &SearchParams,
    allow: &(dyn Fn(EmbeddingId) -> bool + Sync),
    factor: usize,
) -> Result<PostFiltered> {
    let cap = MAX_K.min(index.len().max(1));
    let mut kk = (k * factor.max(1)).clamp(1, cap);
    let mut rounds = 0;
    loop {
        rounds += 1;
        let raw = index.search(q, kk, params, None)?;
        let exhausted = raw.len() < kk;
        let mut hits: Vec<Hit> = raw.into_iter().filter(|h| allow(h.id)).collect();
        if hits.len() >= k || kk >= cap || exhausted {
            let incomplete = hits.len() < k && !exhausted;
            hits.truncate(k);
            return Ok(PostFiltered {
                hits,
                k_final: kk,
                rounds,
                incomplete,
            });
        }
        kk = (kk * 4).min(cap);
    }
}
