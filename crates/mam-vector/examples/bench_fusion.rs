//! P3-6 결합 코어 측정: `fuse_rrf` (2 x 2000 후보)와 `vector_candidates` (flat, 합성 벡터) 지연.
//!
//! 실행: `cargo run -p mam-vector --release --example bench_fusion -- [--n 100000] [--dim 512] [--quant f16] [--runs 50]`
//! 합성 균등 난수 벡터이므로 결과 관련도·실제 임베딩 분포에 대해서는 아무것도 말하지 않는다 (지연 전용).

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::rng::Rng;
use common::{args, print_conditions, stats};
use mam_vector::{
    AnnIndex, AssetHit, CandidatePlan, FlatIndex, Quantization, RrfParams, fuse_rrf, normalized,
    plan_candidates, vector_candidates,
};

fn unit_vec(rng: &mut Rng, dim: usize) -> Vec<f32> {
    let v: Vec<f32> = (0..dim).map(|_| rng.unit() as f32 * 2.0 - 1.0).collect();
    normalized(&v)
}

fn measure(name: &str, runs: usize, mut f: impl FnMut()) {
    f(); // 워밍업
    let samples: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    let s = stats::summarize(&samples);
    println!(
        "| {name} | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        s.n, s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
}

fn main() {
    let a = args::Args::parse();
    let n: usize = a.num("n", 100_000);
    let dim: usize = a.num("dim", 512);
    let runs: usize = a.num("runs", 50);
    let quant = Quantization::parse(a.get("quant").unwrap_or("f16")).unwrap_or(Quantization::F16);

    println!("== bench_fusion ==");
    print_conditions();
    println!(
        "threads: {} / dataset: {n} x {dim} {} synthetic uniform, 1 vector per asset",
        std::thread::available_parallelism().map_or(4, |n| n.get()),
        quant.as_str()
    );

    let mut rng = Rng::new(42);
    let idx = FlatIndex::with_capacity(dim, quant, n);
    for id in 0..n as u64 {
        idx.add(id, &unit_vec(&mut rng, dim)).unwrap();
    }
    let queries: Vec<Vec<f32>> = (0..runs + 1).map(|_| unit_vec(&mut rng, dim)).collect();

    println!("\n| case | runs | P50 ms | P95 ms | P99 ms | max ms |\n|---|---|---|---|---|---|");

    // fuse_rrf: 키워드 2000 + 벡터 2000, 약 절반 겹침
    let keyword: Vec<u64> = (0..2000).map(|i| i * 2).collect();
    let vector: Vec<AssetHit> = (0..2000u64)
        .map(|i| AssetHit {
            asset_id: i * 3,
            score: 1.0 - i as f32 / 4000.0,
            embedding_id: i,
        })
        .collect();
    let p = RrfParams::default();
    measure("fuse_rrf 2x2000", runs, || {
        std::hint::black_box(fuse_rrf(&keyword, &vector, &p));
    });

    let plan: CandidatePlan = plan_candidates(50);
    let asset_of = |e: u64| Some(e);
    let mut qi = 0;
    let mut run = |label: &str, allowed: Option<&[u64]>| {
        qi = 0;
        measure(label, runs, || {
            qi += 1;
            let q = &queries[qi % queries.len()];
            std::hint::black_box(vector_candidates(&idx, q, &plan, allowed, n, &asset_of).unwrap());
        });
    };
    run("vector_candidates no filter (k=400)", None);
    let one: Vec<u64> = (0..n as u64).filter(|i| i % 100 == 3).collect();
    run("vector_candidates 1% allowed (Subset)", Some(&one));
    let ten: Vec<u64> = (0..n as u64).filter(|i| i % 10 == 3).collect();
    run(
        "vector_candidates 10% allowed (FullScanPredicate)",
        Some(&ten),
    );
    let third: Vec<u64> = (0..n as u64).filter(|i| i % 3 == 1).collect();
    run(
        "vector_candidates 33% allowed (FullScanPredicate)",
        Some(&third),
    );
}
