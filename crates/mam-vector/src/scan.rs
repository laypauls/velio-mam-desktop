//! 전수 비교 스캔 코어 — 읽기 전용 슬라이스 위에서 점수 계산·상위 k·스레드 분할.
//! RAM 에 올린 인덱스([`crate::flat::FlatIndex`] `Load`)와 메모리 매핑(`View`), 정답 계산([`exact_topk_f32`])이 같은 코드를 쓴다.

use crate::kernel::{dot_f16, dot_f32, dot_i8, quantize_i8};
use crate::topk::TopK;
use crate::types::{Filter, Hit, normalized};

/// 이 건수 미만이면 스레드를 만들지 않는다 (스레드 생성 비용이 더 크다).
const PARALLEL_MIN: usize = 32 * 1024;

#[derive(Clone, Copy)]
pub(crate) enum Data<'a> {
    F32(&'a [f32]),
    F16(&'a [u16]),
    I8(&'a [i8]),
}

/// 질의 벡터의 정밀도별 준비형 (i8 은 질의도 양자화해 정수 내적을 쓴다).
pub(crate) enum Prepared {
    Float(Vec<f32>),
    Int8(Vec<i8>, f32),
}

/// 읽기 전용 슬라이스 묶음 — RAM 이든 매핑이든 같은 검색 코드를 쓴다.
#[derive(Clone, Copy)]
pub(crate) struct Slices<'a> {
    pub(crate) ids: &'a [u64],
    pub(crate) scales: &'a [f32],
    pub(crate) data: Data<'a>,
    pub(crate) dim: usize,
}

impl Slices<'_> {
    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    pub(crate) fn prepare(&self, q: &[f32]) -> Prepared {
        let qn = normalized(q);
        match self.data {
            Data::I8(_) => {
                let mut qi = vec![0i8; qn.len()];
                let s = quantize_i8(&qn, &mut qi);
                Prepared::Int8(qi, s)
            }
            _ => Prepared::Float(qn),
        }
    }

    #[inline]
    pub(crate) fn score(&self, pq: &Prepared, slot: usize) -> f32 {
        let r = slot * self.dim..(slot + 1) * self.dim;
        match (&self.data, pq) {
            (Data::F32(v), Prepared::Float(q)) => dot_f32(q, &v[r]),
            (Data::F16(v), Prepared::Float(q)) => dot_f16(q, &v[r]),
            (Data::I8(v), Prepared::Int8(q, qs)) => {
                dot_i8(q, &v[r]) as f32 * qs * self.scales[slot]
            }
            // prepare() 가 데이터 종류에 맞춰 만들므로 도달하지 않는다
            _ => f32::NEG_INFINITY,
        }
    }

    pub(crate) fn scan_range(
        &self,
        pq: &Prepared,
        k: usize,
        filter: Option<Filter<'_>>,
        lo: usize,
        hi: usize,
    ) -> TopK {
        let mut top = TopK::new(k);
        for slot in lo..hi {
            let id = self.ids[slot];
            if let Some(f) = filter
                && !f(id)
            {
                continue;
            }
            top.push(id, self.score(pq, slot));
        }
        top
    }

    /// 여러 질의를 한 번의 순회로: 슬롯마다 질의 전부와 내적해 **최대**를 그 벡터의 점수로 쓴다.
    /// 저장 벡터를 질의 수만큼 메모리에서 다시 읽지 않는다 (유사 미디어 질의 프레임 N 개).
    pub(crate) fn scan_range_max(
        &self,
        pqs: &[Prepared],
        k: usize,
        filter: Option<Filter<'_>>,
        lo: usize,
        hi: usize,
    ) -> TopK {
        let mut top = TopK::new(k);
        for slot in lo..hi {
            let id = self.ids[slot];
            if let Some(f) = filter
                && !f(id)
            {
                continue;
            }
            let best = pqs
                .iter()
                .map(|pq| self.score(pq, slot))
                .fold(f32::NEG_INFINITY, f32::max);
            top.push(id, best);
        }
        top
    }

    pub(crate) fn scan_slots(&self, pq: &Prepared, k: usize, slots: &[usize]) -> TopK {
        let mut top = TopK::new(k);
        for &slot in slots {
            top.push(self.ids[slot], self.score(pq, slot));
        }
        top
    }

    pub(crate) fn slot_of(&self, id: u64) -> Option<usize> {
        self.ids.binary_search(&id).ok()
    }
}

/// `n` 개 항목을 `threads` 개 구간으로 나눠 병렬로 훑고 상위 k 를 합친다.
pub(crate) fn par_topk(
    n: usize,
    threads: usize,
    k: usize,
    work: &(dyn Fn(usize, usize) -> TopK + Sync),
) -> TopK {
    let t = if n < PARALLEL_MIN { 1 } else { threads.max(1) };
    if t == 1 {
        return work(0, n);
    }
    let chunk = n.div_ceil(t);
    let mut merged = TopK::new(k);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..t)
            .map(|i| {
                let (lo, hi) = ((i * chunk).min(n), ((i + 1) * chunk).min(n));
                s.spawn(move || work(lo, hi))
            })
            .collect();
        for h in handles {
            merged.merge(h.join().expect("scan thread panicked"));
        }
    });
    merged
}

pub(crate) fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// 정답 계산용: f32 행 데이터(행 i 의 id 는 `ids[i]`)에서 코사인 상위 k. 행은 이미 정규화돼 있어야 한다.
pub fn exact_topk_f32(
    ids: &[u64],
    rows: &[f32],
    dim: usize,
    q: &[f32],
    k: usize,
    threads: usize,
) -> Vec<Hit> {
    let s = Slices {
        ids,
        scales: &[],
        data: Data::F32(rows),
        dim,
    };
    let pq = s.prepare(q);
    par_topk(s.len(), threads, k, &|lo, hi| {
        s.scan_range(&pq, k, None, lo, hi)
    })
    .into_sorted()
}

pub(crate) fn run_subset(
    s: &Slices<'_>,
    q: &[f32],
    k: usize,
    slots: &[usize],
    threads: usize,
) -> Vec<Hit> {
    let pq = s.prepare(q);
    par_topk(slots.len(), threads, k, &|lo, hi| {
        s.scan_slots(&pq, k, &slots[lo..hi])
    })
    .into_sorted()
}
