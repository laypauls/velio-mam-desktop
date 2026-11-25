//! 상위 k 선택용 고정 크기 힙.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use crate::types::Hit;

/// 점수 오름차순(같으면 id 큰 쪽이 "더 나쁨") 정렬 키 — 힙 최솟값이 현재 k 번째 후보다.
#[derive(Clone, Copy, PartialEq)]
struct Cand(Hit);

impl Eq for Cand {}
impl PartialOrd for Cand {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cand {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0
            .score
            .total_cmp(&o.0.score)
            .then_with(|| o.0.id.cmp(&self.0.id))
    }
}

pub struct TopK {
    k: usize,
    heap: BinaryHeap<Reverse<Cand>>,
}

impl TopK {
    pub fn new(k: usize) -> Self {
        Self {
            k,
            heap: BinaryHeap::with_capacity(k + 1),
        }
    }

    #[inline]
    pub fn push(&mut self, id: u64, score: f32) {
        let cand = Cand(Hit { id, score });
        if self.heap.len() < self.k {
            self.heap.push(Reverse(cand));
        } else if let Some(min) = self.heap.peek()
            && cand > min.0
        {
            self.heap.pop();
            self.heap.push(Reverse(cand));
        }
    }

    pub fn merge(&mut self, other: TopK) {
        for Reverse(c) in other.heap {
            self.push(c.0.id, c.0.score);
        }
    }

    /// 점수 내림차순.
    pub fn into_sorted(self) -> Vec<Hit> {
        let mut v: Vec<Cand> = self.heap.into_iter().map(|r| r.0).collect();
        v.sort_by(|a, b| b.cmp(a));
        v.into_iter().map(|c| c.0).collect()
    }
}
