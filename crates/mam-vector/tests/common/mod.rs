//! 통합 테스트 공용 — 소규모 클러스터 벡터 생성기와 엔진 × 정밀도 행렬.
#![allow(dead_code)]

pub mod affinity;

use mam_vector::{
    AnnIndex, Engine, IndexSpec, Quantization, SearchParams, create_index, normalized,
};

pub const DIM: usize = 32;

/// SplitMix64 — 시드 고정 난수.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// [-1, 1)
    pub fn signed(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// 클러스터 중심 8개 + 작은 잡음. 정규화된 `n` 개 벡터를 돌려준다 (행 우선).
pub fn dataset(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = Rng(seed);
    let centers: Vec<Vec<f32>> = (0..8)
        .map(|_| normalized(&(0..dim).map(|_| rng.signed()).collect::<Vec<_>>()))
        .collect();
    (0..n)
        .map(|i| {
            let c = &centers[i % 8];
            let v: Vec<f32> = c.iter().map(|x| x + 0.4 * rng.signed()).collect();
            normalized(&v)
        })
        .collect()
}

pub fn spec(engine: Engine, quant: Quantization) -> IndexSpec {
    IndexSpec {
        engine,
        model_id: "test-model".into(),
        model_version: "1".into(),
        dimension: DIM,
        quantization: quant,
    }
}

/// 모든 (엔진, 정밀도) 조합.
pub fn matrix() -> Vec<IndexSpec> {
    vec![
        spec(Engine::Flat, Quantization::F32),
        spec(Engine::Flat, Quantization::F16),
        spec(Engine::Flat, Quantization::I8),
    ]
}

pub fn label(s: &IndexSpec) -> String {
    format!("{}/{}", s.engine.as_str(), s.quantization.as_str())
}

pub fn build(spec: &IndexSpec, data: &[Vec<f32>]) -> Box<dyn AnnIndex> {
    let idx = create_index(spec, data.len()).unwrap();
    for (i, v) in data.iter().enumerate() {
        idx.add(i as u64 + 1, v).unwrap();
    }
    idx
}

pub fn params() -> SearchParams {
    SearchParams { ef_search: 200 }
}

/// 단순 반복문 정답 (T-vec-13): 코사인 상위 k 의 id.
pub fn naive_topk(data: &[Vec<f32>], q: &[f32], k: usize) -> Vec<u64> {
    let qn = normalized(q);
    let mut s: Vec<(f32, u64)> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let d: f32 = normalized(v).iter().zip(&qn).map(|(a, b)| a * b).sum();
            (d, i as u64 + 1)
        })
        .collect();
    s.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    s.into_iter().take(k).map(|x| x.1).collect()
}

pub fn recall(found: &[u64], truth: &[u64]) -> f64 {
    let hit = found.iter().filter(|id| truth.contains(id)).count();
    hit as f64 / truth.len().max(1) as f64
}
