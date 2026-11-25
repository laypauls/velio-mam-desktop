//! P4 규모 측정 — 자산·임베딩 id 배치와 클러스터(가우시안 혼합) 합성 벡터 생성기.
//!
//! 배치 (자산 1..=A, 이미지 먼저·영상 나중):
//! - 시각 임베딩 id: 이미지 자산 a -> a, 영상 자산은 프레임 5개가 연속 id. 총 `images + videos x 5`
//! - 의미 임베딩 id: `visual + asset_id` (같은 `embeddings` 테이블에 시각 다음으로 적재)
//!
//! 모든 벡터는 (시드, 자산 번호) 로 결정되므로 적재 단계와 정답 계산 단계가 같은 값을 다시 만든다.

use std::ops::Range;

use crate::common::rng::Rng;

pub const SEED: u64 = 20_261_029;
pub const VIS_DIM: usize = 512;
pub const SEM_DIM: usize = 384;
pub const FRAMES: u64 = 5;
/// 질의 벡터는 적재된 자산 번호와 겹치지 않는 키 영역을 쓴다.
const QUERY_KEY_BASE: u64 = 1 << 40;

#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub images: u64,
    pub videos: u64,
}

impl Scale {
    /// `div` 로 나눈 축소판 (예행연습용). 1 이면 전체 규모(이미지 70만 + 영상 30만).
    pub fn new(div: u64) -> Self {
        let d = div.max(1);
        Self {
            images: 700_000 / d,
            videos: 300_000 / d,
        }
    }

    pub fn assets(&self) -> u64 {
        self.images + self.videos
    }

    pub fn visual(&self) -> u64 {
        self.images + self.videos * FRAMES
    }

    pub fn is_video(&self, asset: u64) -> bool {
        asset > self.images
    }

    pub fn visual_asset(&self, eid: u64) -> u64 {
        if eid <= self.images {
            eid
        } else {
            self.images + 1 + (eid - self.images - 1) / FRAMES
        }
    }

    pub fn semantic_eid(&self, asset: u64) -> u64 {
        self.visual() + asset
    }

    pub fn semantic_asset(&self, eid: u64) -> u64 {
        eid - self.visual()
    }

    pub fn visual_eids(&self, asset: u64) -> Range<u64> {
        if asset <= self.images {
            asset..asset + 1
        } else {
            let s = self.images + (asset - self.images - 1) * FRAMES + 1;
            s..s + FRAMES
        }
    }
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter_mut().for_each(|x| *x /= n);
}

/// 군집 중심(정규화 가우시안) + 항목별 잡음. 군집 인기도는 치우쳐 있다(`Rng::skewed`).
/// 영상 프레임은 영상 대표 벡터에 프레임 잡음을 더한 것이다.
pub struct Mixture {
    dim: usize,
    tag: u64,
    clusters: usize,
    centers: Vec<f32>,
    item_sigma: f32,
    frame_sigma: f32,
}

impl Mixture {
    pub fn new(dim: usize, clusters: usize, tag: u64, item_sigma: f32, frame_sigma: f32) -> Self {
        let mut rng = Rng::new(SEED ^ tag);
        let mut centers = vec![0f32; clusters * dim];
        for c in centers.chunks_mut(dim) {
            c.iter_mut().for_each(|x| *x = rng.gauss());
            normalize(c);
        }
        Self {
            dim,
            tag,
            clusters,
            centers,
            item_sigma,
            frame_sigma,
        }
    }

    pub fn visual() -> Self {
        Self::new(VIS_DIM, 1000, 0x51, 0.03, 0.015)
    }

    pub fn semantic() -> Self {
        Self::new(SEM_DIM, 500, 0x5E, 0.035, 0.0)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    fn perturb(base: &[f32], sigma: f32, rng: &mut Rng, out: &mut [f32]) {
        for (o, b) in out.iter_mut().zip(base) {
            *o = b + sigma * rng.gauss();
        }
        normalize(out);
    }

    /// 항목 하나(이미지·영상 대표·텍스트)의 벡터.
    pub fn item(&self, key: u64, out: &mut [f32]) {
        let mut rng = Rng::for_item(SEED ^ self.tag, key);
        let c = rng.skewed(self.clusters);
        let base = &self.centers[c * self.dim..(c + 1) * self.dim];
        Self::perturb(base, self.item_sigma, &mut rng, out);
    }

    /// 영상 프레임 `n` 개 (`out.len() == n x dim`).
    pub fn frames(&self, key: u64, n: usize, out: &mut [f32]) {
        let mut base = vec![0f32; self.dim];
        self.item(key, &mut base);
        for f in 0..n {
            let mut rng = Rng::for_item(SEED ^ self.tag ^ 0xF4A3, key * 8 + f as u64);
            let o = &mut out[f * self.dim..(f + 1) * self.dim];
            Self::perturb(&base, self.frame_sigma, &mut rng, o);
        }
    }
}

fn par_fill(
    lo: u64,
    hi: u64,
    threads: usize,
    f: &(dyn Fn(u64, u64) -> Vec<f32> + Sync),
) -> Vec<f32> {
    let t = threads.max(1) as u64;
    let chunk = (hi - lo).div_ceil(t).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..t)
            .map(|i| (lo + i * chunk, (lo + (i + 1) * chunk).min(hi)))
            .filter(|(l, h)| l < h)
            .map(|(l, h)| s.spawn(move || f(l, h)))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("fill thread panicked"))
            .collect()
    })
}

/// 자산 `[lo, hi)` 의 시각 벡터 행 (임베딩 id 순서).
pub fn fill_visual(mix: &Mixture, sc: &Scale, lo: u64, hi: u64, threads: usize) -> Vec<f32> {
    let dim = mix.dim();
    par_fill(lo, hi, threads, &|l, h| {
        let rows: u64 = (l..h).map(|a| sc.visual_eids(a).count() as u64).sum();
        let mut v = vec![0f32; rows as usize * dim];
        let mut pos = 0;
        for a in l..h {
            if sc.is_video(a) {
                mix.frames(a, FRAMES as usize, &mut v[pos..pos + FRAMES as usize * dim]);
                pos += FRAMES as usize * dim;
            } else {
                mix.item(a, &mut v[pos..pos + dim]);
                pos += dim;
            }
        }
        v
    })
}

/// 자산 `[lo, hi)` 의 의미 벡터 행.
pub fn fill_semantic(mix: &Mixture, lo: u64, hi: u64, threads: usize) -> Vec<f32> {
    let dim = mix.dim();
    par_fill(lo, hi, threads, &|l, h| {
        let mut v = vec![0f32; (h - l) as usize * dim];
        for (i, a) in (l..h).enumerate() {
            mix.item(a, &mut v[i * dim..(i + 1) * dim]);
        }
        v
    })
}

pub fn item_queries(mix: &Mixture, n: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| {
            let mut v = vec![0f32; mix.dim()];
            mix.item(QUERY_KEY_BASE + i as u64, &mut v);
            v
        })
        .collect()
}

/// 유사 미디어 질의: 무작위 영상 자산 `n` 개의 (자산 id, 프레임 5개).
pub fn video_queries(mix: &Mixture, sc: &Scale, n: usize) -> Vec<(u64, Vec<Vec<f32>>)> {
    let dim = mix.dim();
    let mut rng = Rng::new(SEED ^ 0x51D);
    (0..n)
        .map(|_| {
            let a = sc.images + 1 + rng.below(sc.videos as usize) as u64;
            let mut fr = vec![0f32; FRAMES as usize * dim];
            mix.frames(a, FRAMES as usize, &mut fr);
            (a, fr.chunks(dim).map(<[f32]>::to_vec).collect())
        })
        .collect()
}

/// 자산 `a` 가 허용 `pct`% 집합에 속하는가 (벡터와 무상관인 고정 시드 무작위 부분집합).
pub fn selected(asset: u64, pct: u64) -> bool {
    Rng::for_item(SEED ^ 0xF117, asset).next_u64() % 100 < pct
}

pub fn allowed_assets(sc: &Scale, pct: u64) -> Vec<u64> {
    (1..=sc.assets()).filter(|&a| selected(a, pct)).collect()
}

/// 허용 자산의 시각 임베딩 id 전부 (영상은 프레임 5개씩).
pub fn allowed_eids(sc: &Scale, pct: u64) -> Vec<u64> {
    allowed_assets(sc, pct)
        .into_iter()
        .flat_map(|a| sc.visual_eids(a))
        .collect()
}
