//! 합성 벡터 모델 (Q3) — 클러스터 구조 + 영상 프레임 간 상관, 정규화, 시드 고정.
//!
//! 자산 벡터 = normalize(클러스터 중심 + A × 잡음), 프레임 벡터 = normalize(자산 벡터 + B × 잡음).
//! 잡음은 공유 저차원 부분공간 성분(내재 차원을 낮춤)과 등방 성분의 합이다. 실제 CLIP 류 분포와 다르다 —
//! 엔진·양자화·저장 형식 **비교용**이며 절대 Recall·지연은 P1-6 의 실제 임베딩으로 재측정해야 한다.

use super::rng::Rng;

/// 자산 벡터가 클러스터 중심에서 벗어나는 정도 (cos(자산, 중심) ≈ 0.71).
const ASSET_SPREAD: f32 = 1.0;
/// 프레임이 자산 벡터에서 벗어나는 정도 (cos(프레임, 자산) ≈ 0.94).
const FRAME_SPREAD: f32 = 0.35;
/// 공유 저차원 부분공간의 차수.
const LOW_RANK: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Clustered,
    /// 대조군: 클러스터 없는 균일 가우시안 방향.
    Uniform,
}

pub struct Model {
    pub dim: usize,
    pub kind: Kind,
    clusters: usize,
    centers: Vec<f32>,
    basis: Vec<f32>,
}

fn unit_gaussian(rng: &mut Rng, out: &mut [f32]) {
    for x in out.iter_mut() {
        *x = rng.gauss();
    }
    let n = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    out.iter_mut().for_each(|x| *x /= n);
}

pub fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}

impl Model {
    pub fn new(dim: usize, clusters: usize, seed: u64, kind: Kind) -> Self {
        let mut rng = Rng::new(seed ^ 0xC1u64 << 48);
        let mut centers = vec![0f32; clusters * dim];
        for c in centers.chunks_mut(dim) {
            unit_gaussian(&mut rng, c);
        }
        let mut basis = vec![0f32; LOW_RANK * dim];
        for b in basis.chunks_mut(dim) {
            unit_gaussian(&mut rng, b);
        }
        Self {
            dim,
            kind,
            clusters,
            centers,
            basis,
        }
    }

    pub fn clusters(&self) -> usize {
        self.clusters
    }

    /// 크기 ≈ 1 인 잡음 벡터: 0.75 × 저차원 + 0.66 × 등방.
    fn noise(&self, rng: &mut Rng, out: &mut [f32]) {
        let d = self.dim;
        let scale_low = 0.75 / (LOW_RANK as f32).sqrt();
        out.iter_mut().for_each(|x| *x = 0.0);
        for b in self.basis.chunks(d) {
            let z = rng.gauss() * scale_low;
            for (o, w) in out.iter_mut().zip(b) {
                *o += z * w;
            }
        }
        let scale_full = 0.66 / (d as f32).sqrt();
        for o in out.iter_mut() {
            *o += rng.gauss() * scale_full;
        }
    }

    /// 이미지 1장 / 영상 1개의 대표 벡터. 질의 생성에도 쓴다.
    pub fn asset_vec(&self, rng: &mut Rng, out: &mut [f32]) {
        match self.kind {
            Kind::Uniform => {
                unit_gaussian(rng, out);
            }
            Kind::Clustered => {
                let c = rng.skewed(self.clusters);
                let center = &self.centers[c * self.dim..(c + 1) * self.dim];
                let mut noise = vec![0f32; self.dim];
                self.noise(rng, &mut noise);
                for ((o, c), n) in out.iter_mut().zip(center).zip(&noise) {
                    *o = c + ASSET_SPREAD * n;
                }
                normalize(out);
            }
        }
    }

    /// 같은 영상의 프레임: 자산 벡터 + 작은 잡음.
    pub fn frame_vec(&self, rng: &mut Rng, asset: &[f32], out: &mut [f32]) {
        match self.kind {
            Kind::Uniform => {
                let mut g = vec![0f32; self.dim];
                unit_gaussian(rng, &mut g);
                for ((o, a), n) in out.iter_mut().zip(asset).zip(&g) {
                    *o = a + FRAME_SPREAD * n;
                }
            }
            Kind::Clustered => {
                let mut noise = vec![0f32; self.dim];
                self.noise(rng, &mut noise);
                for ((o, a), n) in out.iter_mut().zip(asset).zip(&noise) {
                    *o = a + FRAME_SPREAD * n;
                }
            }
        }
        normalize(out);
    }
}

/// 영상 프레임 수: 길이 비례(6초당 1프레임), 1~10. 이미지 1, 음성 0 (가정값 — 작업 문서 Q4).
pub fn frames_for(media_type: &str, duration_ms: Option<i64>) -> usize {
    match media_type {
        "image" => 1,
        "video" => {
            let secs = duration_ms.unwrap_or(0).max(0) as f64 / 1000.0;
            ((secs / 6.0).ceil() as usize).clamp(1, 10)
        }
        _ => 0,
    }
}
