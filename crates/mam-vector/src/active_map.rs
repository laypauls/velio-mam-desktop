//! 활성 `embedding_id -> asset_id` 맵의 압축 표현.
//!
//! `embedding_id` 오름차순 평행 배열 두 개(16B/행)와 존재 비트맵이다. 220만 행 약 35MB —
//! `HashMap<u64, u64>` 약 68MB 의 절반이고, 질의 스캔 중 "활성인가" 판정은 비트맵으로 O(1) 이다.
//! 개별 조회(`asset_of`)는 이진 탐색이라 최종 후보(수백 건)에만 쓴다 — 스캔 중 벡터마다 부르지 말 것.

use crate::idset::IdSet;
use crate::types::EmbeddingId;

#[derive(Clone)]
pub struct ActiveMap {
    ids: Vec<EmbeddingId>,
    assets: Vec<u64>,
    present: IdSet,
}

impl ActiveMap {
    /// `ids` 는 오름차순이어야 한다 (`assets[i]` 가 `ids[i]` 의 자산).
    pub(crate) fn from_sorted(ids: Vec<EmbeddingId>, assets: Vec<u64>) -> Self {
        debug_assert_eq!(ids.len(), assets.len());
        debug_assert!(ids.windows(2).all(|w| w[0] < w[1]));
        let present = IdSet::from_ids(&ids);
        Self {
            ids,
            assets,
            present,
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// 활성 임베딩 id, 오름차순.
    pub fn ids(&self) -> &[EmbeddingId] {
        &self.ids
    }

    /// 활성 여부 (비트맵, O(1)) — 스캔 중 판정용.
    pub fn contains(&self, id: EmbeddingId) -> bool {
        self.present.contains(id)
    }

    pub(crate) fn present(&self) -> &IdSet {
        &self.present
    }

    /// 활성 벡터의 자산. 폐기됐거나 없으면 `None` (이진 탐색).
    pub fn asset_of(&self, id: EmbeddingId) -> Option<u64> {
        if !self.present.contains(id) {
            return None;
        }
        self.ids.binary_search(&id).ok().map(|i| self.assets[i])
    }

    /// (임베딩 id, 자산 id), 임베딩 id 오름차순.
    pub fn iter(&self) -> impl Iterator<Item = (EmbeddingId, u64)> + '_ {
        self.ids.iter().copied().zip(self.assets.iter().copied())
    }

    /// 허용 자산들의 활성 임베딩 id (임베딩 id 오름차순). 맵을 한 번 훑으므로 허용 자산 수와 무관하게
    /// 행 수에 비례하지만 순차 접근이라 220만 행에 수 ms 다.
    pub fn embeddings_of_assets(&self, allowed_assets: &[u64]) -> Vec<EmbeddingId> {
        self.embeddings_in(&IdSet::from_ids(allowed_assets))
    }

    /// [`Self::embeddings_of_assets`] 의 집합 입력판 — 호출자가 만들어 둔(캐시한) 자산 집합을 그대로 쓴다.
    pub fn embeddings_in(&self, allowed_assets: &IdSet) -> Vec<EmbeddingId> {
        let set = allowed_assets;
        self.iter()
            .filter(|&(_, a)| set.contains(a))
            .map(|(e, _)| e)
            .collect()
    }

    /// `touched` 자산의 행을 모두 버리고 `rows`(그 자산들의 현재 활성 행, 임베딩 id 오름차순)로 바꾼다.
    /// 한 번의 병합 순회 — 220만 행에 수십 ms 이하.
    pub(crate) fn replace_assets(&mut self, touched: &IdSet, rows: Vec<(EmbeddingId, u64)>) {
        let cap = self.ids.len() + rows.len();
        let (mut ids, mut assets) = (Vec::with_capacity(cap), Vec::with_capacity(cap));
        let mut new = rows.into_iter().peekable();
        for (&e, &a) in self.ids.iter().zip(&self.assets) {
            if touched.contains(a) {
                continue;
            }
            while let Some(&(re, ra)) = new.peek().filter(|r| r.0 < e) {
                ids.push(re);
                assets.push(ra);
                new.next();
            }
            ids.push(e);
            assets.push(a);
        }
        for (re, ra) in new {
            ids.push(re);
            assets.push(ra);
        }
        self.present = IdSet::from_ids(&ids);
        self.ids = ids;
        self.assets = assets;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_amap_01_lookup_replace_and_asset_expansion() {
        // Arrange
        let mut m = ActiveMap::from_sorted(vec![2, 3, 5, 8], vec![10, 10, 11, 12]);
        // Act + Assert: 조회
        assert_eq!(m.asset_of(5), Some(11));
        assert_eq!(m.asset_of(4), None);
        assert!(m.contains(8) && !m.contains(1));
        assert_eq!(m.embeddings_of_assets(&[10, 12, 99]), vec![2, 3, 8]);
        // Act: 자산 10 의 행(2, 3)을 4 하나로 바꾸고, 새 자산 13 의 9 를 추가 (자산 12 의 8 은 폐기돼 rows 에 없음)
        m.replace_assets(&IdSet::from_ids(&[10, 12, 13]), vec![(4, 10), (9, 13)]);
        // Assert
        assert_eq!(m.ids(), &[4, 5, 9]);
        assert_eq!(m.asset_of(3), None);
        assert_eq!(m.asset_of(8), None);
        assert_eq!(m.asset_of(9), Some(13));
        assert_eq!(m.embeddings_of_assets(&[10]), vec![4]);
        assert_eq!(m.len(), 3);
    }
}
