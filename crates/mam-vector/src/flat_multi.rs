//! 다중 질의 단일 순회 검색 — 유사 미디어(기준 영상의 프레임 N 개)용.

use crate::error::Result;
use crate::flat::FlatIndex;
use crate::scan::par_topk;
use crate::types::{Filter, Hit, validate_query};

impl FlatIndex {
    /// 질의 벡터 `qs` 전부와의 코사인 중 **최대**를 각 저장 벡터의 점수로 보고 상위 `k` 를 돌려준다
    /// (점수 내림차순, 같으면 id 오름차순 — `search` 와 같은 순서 규칙). 인덱스를 한 번만 훑는다.
    /// 각 질의의 점수는 `search` 의 점수와 비트까지 같다 (같은 커널).
    pub fn search_multi(
        &self,
        qs: &[Vec<f32>],
        k: usize,
        filter: Option<Filter<'_>>,
    ) -> Result<Vec<Hit>> {
        for q in qs {
            validate_query(self.dim(), q, k)?;
        }
        self.with_slices(|s, threads| {
            let pqs: Vec<_> = qs.iter().map(|q| s.prepare(q)).collect();
            par_topk(s.len(), threads, k, &|lo, hi| {
                s.scan_range_max(&pqs, k, filter, lo, hi)
            })
            .into_sorted()
        })
    }
}
