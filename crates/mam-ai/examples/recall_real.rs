//! P1-3 후속 - **실제 CLIP 임베딩**으로 채택 벡터 엔진(전수 비교 f16/i8)의 Recall@10 을 f32 정답과 비교한다.
//!
//! `recall_real --vecs <embed_dataset out> [--queries 1000] [--threads 6]`
//! 질의 3종: (a) 영어 캡션 텍스트 벡터(교차 모달, 실제 검색과 같다) (b) 한/영 카테고리 질의 160개 (c) 이미지->이미지 (자기 자신 포함 색인).
//! 색인: COCO 원본·반전·크롭 15,000 + 합성 3,000 = 18,000 벡터 (512차원, 정규화됨).

#[path = "common/mod.rs"]
mod common;

use std::path::PathBuf;

use common::{Args, fail};
use mam_vector::{AnnIndex, FlatIndex, Quantization, SearchParams, exact_topk_f32};

const DIM: usize = 512;

fn read(dir: &std::path::Path, name: &str) -> Vec<Vec<f32>> {
    let b = std::fs::read(dir.join(name)).unwrap_or_else(|e| fail(&format!("{name}: {e}")));
    b.chunks_exact(DIM * 4)
        .map(|r| {
            r.chunks_exact(4)
                .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                .collect()
        })
        .collect()
}

struct QuerySet {
    name: &'static str,
    queries: Vec<Vec<f32>>,
    /// 정답 상위 100 (f32 전수)
    truth: Vec<Vec<u64>>,
}

fn recall(found: &[u64], truth: &[u64], k: usize) -> f64 {
    let t = &truth[..k.min(truth.len())];
    found.iter().take(k).filter(|id| t.contains(id)).count() as f64 / t.len() as f64
}

/// 질의별 Recall@10, Recall@100 평균과 top-1 일치율.
fn measure(idx: &dyn AnnIndex, set: &QuerySet, params: &SearchParams) -> (f64, f64, f64) {
    let (mut r10, mut r100, mut top1) = (0.0, 0.0, 0.0);
    for (q, t) in set.queries.iter().zip(&set.truth) {
        let hits = idx.search(q, 100, params, None).expect("search");
        let ids: Vec<u64> = hits.iter().map(|h| h.id).collect();
        r10 += recall(&ids, t, 10);
        r100 += recall(&ids, t, 100);
        top1 += (ids.first() == t.first()) as usize as f64;
    }
    let n = set.queries.len() as f64;
    (r10 / n, r100 / n, top1 / n)
}

fn main() {
    let a = Args::parse();
    let dir = PathBuf::from(a.require("vecs"));
    let threads: usize = a.num("threads", 6);
    let nq: usize = a.num("queries", 1000);
    let images = read(&dir, "image_vectors.f32");
    let n = images.len();
    let ids: Vec<u64> = (0..n as u64).collect();
    let flat_rows: Vec<f32> = images.iter().flatten().copied().collect();

    let mut sets = Vec::new();
    let caps: Vec<Vec<f32>> = read(&dir, "caption_vectors.f32")
        .into_iter()
        .step_by(5)
        .take(nq)
        .collect();
    let mut cats = read(&dir, "class_vectors_en.f32");
    cats.extend(read(&dir, "class_vectors_ko.f32"));
    let img_q: Vec<Vec<f32>> = images
        .iter()
        .step_by((n / nq).max(1))
        .take(nq)
        .cloned()
        .collect();
    for (name, queries) in [
        ("영어 캡션 -> 이미지", caps),
        ("카테고리 한/영 -> 이미지", cats),
        ("이미지 -> 이미지", img_q),
    ] {
        let truth = queries
            .iter()
            .map(|q| {
                exact_topk_f32(&ids, &flat_rows, DIM, q, 100, threads)
                    .iter()
                    .map(|h| h.id)
                    .collect()
            })
            .collect();
        sets.push(QuerySet {
            name,
            queries,
            truth,
        });
    }
    println!(
        "index vectors: {n} (dim {DIM}); queries: {}",
        sets.iter()
            .map(|s| format!("{}={}", s.name, s.queries.len()))
            .collect::<Vec<_>>()
            .join(", ")
    );

    println!(
        "\n| 엔진 | 정밀도 | 질의 종류 | Recall@10 | Recall@100 | top-1 일치 |\n|---|---|---|---:|---:|---:|"
    );
    let mut candidates: Vec<(String, Box<dyn AnnIndex>, Vec<SearchParams>)> = Vec::new();
    for q in [Quantization::F32, Quantization::F16, Quantization::I8] {
        let idx = FlatIndex::with_capacity(DIM, q, n);
        idx.set_threads(threads);
        for (i, v) in images.iter().enumerate() {
            idx.add(i as u64, v).expect("add");
        }
        candidates.push((
            format!("flat {}", q.as_str()),
            Box::new(idx),
            vec![SearchParams::default()],
        ));
    }

    for (name, idx, params) in &candidates {
        for p in params {
            for set in &sets {
                let (r10, r100, t1) = measure(&**idx, set, p);
                let label = if params.len() > 1 {
                    format!("{name} ef{}", p.ef_search)
                } else {
                    name.clone()
                };
                println!(
                    "| {label} | {} | {} | {r10:.4} | {r100:.4} | {t1:.3} |",
                    idx.quantization().as_str(),
                    set.name
                );
            }
        }
    }
}
