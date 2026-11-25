//! D5: 유사 미디어 질의 — 기준 자산의 프레임 벡터 각각으로 찾고 자산별 최대 유사도로 묶는다.
//!
//! 기준 자산·중복본(`exclude`)은 **검색 중** 걸러 후보를 잃지 않는다. 정렬은 (유사도 내림차순, asset_id 오름차순)이고
//! cursor 는 마지막 항목의 (유사도, asset_id) 다.

use std::collections::HashMap;

use crate::error::{Result, VectorError};
use crate::flat::FlatIndex;
use crate::types::{AnnIndex, EmbeddingId, MAX_K};

/// 한 페이지 최대 항목 수.
pub const MAX_SIMILAR_LIMIT: usize = 1_000;

/// 페이지 이어받기 위치 — 마지막으로 돌려준 (유사도 비트, asset_id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimilarCursor {
    score_bits: u32,
    asset_id: u64,
}

impl SimilarCursor {
    /// 불투명 문자열 (`s1.<score hex>.<asset hex>`).
    pub fn encode(&self) -> String {
        format!("s1.{:08x}.{:x}", self.score_bits, self.asset_id)
    }

    pub fn decode(s: &str) -> Result<Self> {
        let bad = || VectorError::InvalidInput(format!("malformed similar cursor: {s:?}"));
        let mut it = s.split('.');
        let (Some("s1"), Some(a), Some(b), None) = (it.next(), it.next(), it.next(), it.next())
        else {
            return Err(bad());
        };
        if a.len() != 8 {
            return Err(bad());
        }
        let score_bits = u32::from_str_radix(a, 16).map_err(|_| bad())?;
        let asset_id = u64::from_str_radix(b, 16).map_err(|_| bad())?;
        let score = f32::from_bits(score_bits);
        if !(0.0..=1.0).contains(&score) {
            return Err(bad());
        }
        Ok(Self {
            score_bits,
            asset_id,
        })
    }

    fn from_item(asset_id: u64, score: f32) -> Self {
        Self {
            score_bits: score.to_bits(),
            asset_id,
        }
    }

    /// 정렬 순서상 `(score, id)` 가 cursor 뒤인가.
    fn is_before(&self, score: f32, id: u64) -> bool {
        let c = f32::from_bits(self.score_bits);
        score < c || (score == c && id > self.asset_id)
    }
}

pub struct SimilarQuery<'a> {
    /// 기준 자산의 프레임 벡터들 (비어 있으면 오류).
    pub frames: &'a [Vec<f32>],
    /// 결과에서 뺄 자산 — 기준 자산 자신과 같은 해시 중복본 (호출자 제공).
    pub exclude: &'a [u64],
    pub limit: usize,
    pub cursor: Option<SimilarCursor>,
    /// 최소 유사도(0..=1, 반환되는 클램프 값과 같은 기준, 같으면 포함). 미만이 나오면 거기서 끊고 `next` 는 없다.
    /// 0 = 제한 없음.
    pub min_score: f32,
}

#[derive(Debug)]
pub struct SimilarPage {
    /// (asset_id, 코사인 유사도 0..1 로 클램프) — 유사도 내림차순.
    pub items: Vec<(u64, f32)>,
    /// 다음 페이지가 있으면 그 시작 위치.
    pub next: Option<SimilarCursor>,
}

fn clamp01(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

/// 자산별 최대 유사도 순으로 `limit` 개.
///
/// 인덱스를 **한 번만** 훑는다 (`FlatIndex::search_multi`: 저장 벡터마다 모든 질의 프레임과 내적해 최대를 점수로).
/// 스캔 중에는 벡터마다 맵을 조회하지 않고 점수만 계산하며, 기준 자산·중복본 제외와 활성 여부는 상위 k 에 대해
/// `asset_of` 로 **스캔 뒤** 걸러낸다 (이전 구현은 프레임마다 스캔하고 벡터마다 해시 조회를 했다 — P4 측정).
/// 걸러낸 뒤 순서가 확정된 항목(점수 > 비필터 k 번째 점수)이 `limit + 1` 개가 될 때까지 k 를 키워 다시 훑는다.
/// 첫 k 는 제외 자산 수 x 프레임 수를 미리 더해 중복본이 많아도 다시 훑지 않게 한다.
/// `MAX_K` 를 넘는 상위가 전부 제외·폐기된 극단 상황에서는 근사다 (이전 구현과 같은 한계).
pub fn find_similar(
    index: &FlatIndex,
    q: &SimilarQuery<'_>,
    asset_of: &(dyn Fn(EmbeddingId) -> Option<u64> + Sync),
) -> Result<SimilarPage> {
    if q.frames.is_empty() {
        return Err(VectorError::InvalidInput("no query frames".into()));
    }
    if q.limit == 0 || q.limit > MAX_SIMILAR_LIMIT {
        return Err(VectorError::InvalidInput(format!(
            "limit must be in 1..={MAX_SIMILAR_LIMIT}, got {}",
            q.limit
        )));
    }
    if !(0.0..=1.0).contains(&q.min_score) {
        return Err(VectorError::InvalidInput(format!(
            "min_score must be in 0..=1, got {}",
            q.min_score
        )));
    }
    let mut excluded: Vec<u64> = q.exclude.to_vec();
    excluded.sort_unstable();
    excluded.dedup();
    let cap = MAX_K.min(index.len().max(1));
    let mut k = ((q.limit + 1) * 4)
        .saturating_add(excluded.len().saturating_mul(q.frames.len()))
        .min(cap);
    loop {
        let hits = index.search_multi(q.frames, k, None)?;
        // 확정 하한: 상위 k 에 못 든 벡터는 이 점수 이하다. k 보다 적게 돌아왔으면 전부 본 것이다.
        let threshold = if hits.len() >= k {
            clamp01(hits[hits.len() - 1].score)
        } else {
            f32::NEG_INFINITY
        };
        let mut best: HashMap<u64, f32> = HashMap::new();
        for h in hits {
            let Some(a) = asset_of(h.id) else { continue };
            if excluded.binary_search(&a).is_ok() {
                continue;
            }
            let s = clamp01(h.score);
            best.entry(a).and_modify(|b| *b = b.max(s)).or_insert(s);
        }
        let mut items: Vec<(u64, f32)> = best
            .into_iter()
            .filter(|&(a, s)| q.cursor.is_none_or(|c| c.is_before(s, a)))
            .collect();
        items.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        // 상위 k 의 꼴찌가 이미 최소 유사도 미만이면 더 훑어도 기준 이상 항목이 나오지 않는다 (조기 종료)
        let below_min = threshold < q.min_score;
        let done = threshold == f32::NEG_INFINITY || k >= cap || below_min;
        if !done {
            items.retain(|&(_, s)| s > threshold);
        }
        // 정렬이 내림차순이라 기준 미만은 뒤쪽 전부다 — 잘라내고 다음 페이지는 없다
        let cut = items.partition_point(|&(_, s)| s >= q.min_score);
        let thresholded = cut < items.len();
        items.truncate(cut);
        if items.len() > q.limit || done || thresholded {
            let more = items.len() > q.limit;
            items.truncate(q.limit);
            let next = if more {
                items.last().map(|&(a, s)| SimilarCursor::from_item(a, s))
            } else {
                None
            };
            return Ok(SimilarPage { items, next });
        }
        k = (k * 2).min(cap);
    }
}
