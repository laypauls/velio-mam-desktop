//! 이전 `find_similar` 알고리즘 사본 (P4 개선 전) — 새 구현과의 동등성 테스트와 전/후 측정용.
//! 프레임마다 한 번씩 필터 스캔(벡터마다 `asset_of` + 제외 해시 조회)하고 자산별 최대로 묶는다. cursor 는 다루지 않는다.
#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use mam_vector::{AnnIndex, EmbeddingId, MAX_K, Result, SearchParams};

fn clamp01(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// 자산별 최대 유사도 상위 `limit + 1` 개까지 (더 있는지 보려고 1개 더) — (유사도 내림, asset_id 오름).
pub fn legacy_find_similar(
    index: &dyn AnnIndex,
    frames: &[Vec<f32>],
    exclude: &[u64],
    limit: usize,
    asset_of: &(dyn Fn(EmbeddingId) -> Option<u64> + Sync),
) -> Result<Vec<(u64, f32)>> {
    let excluded: HashSet<u64> = exclude.iter().copied().collect();
    let keep = |id: EmbeddingId| asset_of(id).is_some_and(|a| !excluded.contains(&a));
    let params = SearchParams::default();
    let cap = MAX_K.min(index.len().max(1));
    let mut k = ((limit + 1) * 4).min(cap);
    loop {
        let mut best: HashMap<u64, f32> = HashMap::new();
        let mut threshold = f32::NEG_INFINITY;
        for f in frames {
            let hits = index.search(f, k, &params, Some(&keep))?;
            if hits.len() >= k {
                threshold = threshold.max(clamp01(hits[hits.len() - 1].score));
            }
            for h in hits {
                let Some(a) = asset_of(h.id) else { continue };
                let s = clamp01(h.score);
                best.entry(a).and_modify(|b| *b = b.max(s)).or_insert(s);
            }
        }
        let mut items: Vec<(u64, f32)> = best.into_iter().collect();
        items.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let done = threshold == f32::NEG_INFINITY || k >= cap;
        if !done {
            items.retain(|&(_, s)| s > threshold);
        }
        if items.len() > limit || done {
            items.truncate(limit);
            return Ok(items);
        }
        k = (k * 2).min(cap);
    }
}
