//! 정수 id 집합 — 스캔 중 벡터마다 호출되는 판정(`contains`)을 해시 없이 O(1) 로 한다.
//!
//! 해시 집합은 SipHash 때문에 벡터당 수십 ns 가 들어 스캔 시간을 두 배로 만들었다 (P4 측정).
//! id 범위가 허용 개수에 비해 너무 넓지 않으면 비트맵(범위/8 바이트)을 쓰고, 아니면 해시 집합으로 되돌아간다.

use std::collections::HashSet;

/// 비트맵이 쓸 수 있는 범위 상한 = `개수 x 64 + 2^20` 비트 (바이트로 `개수 x 8 + 128KB` 이하).
const SPARSE_FACTOR: u64 = 64;
const SPARSE_SLACK: u64 = 1 << 20;

enum Repr {
    Bits { base: u64, words: Vec<u64> },
    Hash(HashSet<u64>),
}

pub struct IdSet {
    repr: Repr,
    len: usize,
}

impl IdSet {
    /// 중복은 한 번만 센다. 입력은 정렬돼 있지 않아도 된다.
    pub fn from_ids(ids: &[u64]) -> Self {
        let (Some(&min), Some(&max)) = (ids.iter().min(), ids.iter().max()) else {
            return Self {
                repr: Repr::Bits {
                    base: 0,
                    words: Vec::new(),
                },
                len: 0,
            };
        };
        let range = max - min;
        let budget = (ids.len() as u64)
            .saturating_mul(SPARSE_FACTOR)
            .saturating_add(SPARSE_SLACK);
        if range >= budget {
            let set: HashSet<u64> = ids.iter().copied().collect();
            let len = set.len();
            return Self {
                repr: Repr::Hash(set),
                len,
            };
        }
        let mut words = vec![0u64; (range / 64 + 1) as usize];
        for &id in ids {
            let off = id - min;
            words[(off >> 6) as usize] |= 1 << (off & 63);
        }
        let len = words.iter().map(|w| w.count_ones() as usize).sum();
        Self {
            repr: Repr::Bits { base: min, words },
            len,
        }
    }

    #[inline]
    pub fn contains(&self, id: u64) -> bool {
        match &self.repr {
            Repr::Bits { base, words } => id
                .checked_sub(*base)
                .and_then(|off| {
                    words
                        .get((off >> 6) as usize)
                        .map(|w| (w >> (off & 63)) & 1 == 1)
                })
                .unwrap_or(false),
            Repr::Hash(s) => s.contains(&id),
        }
    }

    /// 서로 다른 id 의 수.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Clone for IdSet {
    fn clone(&self) -> Self {
        Self {
            repr: match &self.repr {
                Repr::Bits { base, words } => Repr::Bits {
                    base: *base,
                    words: words.clone(),
                },
                Repr::Hash(s) => Repr::Hash(s.clone()),
            },
            len: self.len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_idset_01_dense_and_sparse_agree_with_hashset() {
        // Arrange: 조밀 / 아주 성긴(해시 경로) / 경계값(0, u64::MAX)
        let dense: Vec<u64> = (10..200).filter(|i| i % 3 != 0).collect();
        let sparse = vec![1u64, 1 << 40, 7, u64::MAX, 0, 7];
        for ids in [dense, sparse] {
            // Act
            let set = IdSet::from_ids(&ids);
            let truth: HashSet<u64> = ids.iter().copied().collect();
            // Assert
            assert_eq!(set.len(), truth.len());
            for probe in (0..300).chain([1 << 40, u64::MAX, u64::MAX - 1]) {
                assert_eq!(set.contains(probe), truth.contains(&probe), "{probe}");
            }
        }
    }

    #[test]
    fn t_idset_02_empty_set_contains_nothing() {
        let set = IdSet::from_ids(&[]);
        assert!(set.is_empty());
        assert!(!set.contains(0) && !set.contains(u64::MAX));
    }
}
