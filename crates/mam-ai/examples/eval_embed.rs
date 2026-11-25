//! 임베딩 정확도 평가 (D7) - 합성 이미지(색·도형) x 한국어/영어 질의, e5 의미 검색, 양자화 모델과의 코사인 일치도.
//!
//! `eval_embed --vision <vision.onnx> --text <model.onnx> --tokenizer <tok> --dense <dense> --e5 <model.onnx> --e5-tokenizer <tok>
//!   [--vision-alt <int8.onnx>] [--text-alt <int8.onnx>] [--e5-alt <int8.onnx>] --threads 6`
//!
//! 합성 이미지는 실제 사진보다 쉬우므로 정확도가 높게 나온다 - 결과는 **상한 참고치**다.

#[path = "common/mod.rs"]
mod common;

use std::path::Path;

use common::synth::{COLORS, SHAPES, base_set};
use common::{Args, fail};
use image::DynamicImage;
use mam_ai::local::embed::{ClipTextEmbedder, E5Embedder, EmbedConfig, ImageEmbedder, TextRole};

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn main() {
    let a = Args::parse();
    let cfg = EmbedConfig {
        threads: a.num("threads", 4),
        batch_size: 8,
    };
    let imgs: Vec<(DynamicImage, usize, usize)> = base_set()
        .into_iter()
        .map(|(i, c, s)| (DynamicImage::ImageRgb8(i), c, s))
        .collect();
    let refs: Vec<&DynamicImage> = imgs.iter().map(|(i, _, _)| i).collect();
    println!("synthetic images: {}", imgs.len());

    // 1) 이미지 임베딩 + 텍스트 질의 -> 이미지 검색
    let vision = ImageEmbedder::load(Path::new(a.require("vision")), cfg)
        .unwrap_or_else(|e| fail(&e.to_string()));
    let iv = vision.embed_images(&refs).expect("image embed");
    let text = ClipTextEmbedder::load(
        Path::new(a.require("text")),
        Path::new(a.require("tokenizer")),
        Path::new(a.require("dense")),
        cfg,
    )
    .unwrap_or_else(|e| fail(&e.to_string()));

    let mut queries: Vec<(String, &str, usize, usize)> = Vec::new(); // (text, lang, color, shape)
    for (ci, (ko_c, en_c, ..)) in COLORS.iter().enumerate() {
        for (si, (_, ko_s, en_s)) in SHAPES.iter().enumerate() {
            queries.push((format!("{ko_c} {ko_s}"), "ko", ci, si));
            queries.push((format!("a {en_c} {en_s}"), "en", ci, si));
        }
    }
    let q_text: Vec<&str> = queries.iter().map(|q| q.0.as_str()).collect();
    let qv = text.embed_texts(&q_text).expect("text embed");

    println!(
        "\n## 텍스트 -> 이미지 (합성 색·도형 {} 장, 질의 {} 개)\n",
        imgs.len(),
        queries.len()
    );
    println!(
        "| 질의 언어 | 질의 수 | P@1 (색+도형 정확) | Recall@5 | 색만 맞은 top-1 | 도형만 맞은 top-1 |\n|---|---:|---:|---:|---:|---:|"
    );
    for lang in ["ko", "en"] {
        let (mut n, mut p1, mut r5, mut color_only, mut shape_only) = (0, 0, 0, 0, 0);
        for (q, v) in queries.iter().zip(&qv) {
            if q.1 != lang {
                continue;
            }
            n += 1;
            let mut order: Vec<usize> = (0..iv.len()).collect();
            order.sort_by(|&x, &y| dot(v, &iv[y]).partial_cmp(&dot(v, &iv[x])).unwrap());
            let rel = |i: usize| imgs[i].1 == q.2 && imgs[i].2 == q.3;
            if rel(order[0]) {
                p1 += 1;
            } else if imgs[order[0]].1 == q.2 {
                color_only += 1;
            } else if imgs[order[0]].2 == q.3 {
                shape_only += 1;
            }
            if order[..5].iter().any(|&i| rel(i)) {
                r5 += 1;
            }
        }
        println!(
            "| {lang} | {n} | {:.3} | {:.3} | {color_only} | {shape_only} |",
            p1 as f64 / n as f64,
            r5 as f64 / n as f64
        );
    }
    // 대표 질의 하나: "빨간 원" top-3
    let ko_red_circle = queries.iter().position(|q| q.0 == "빨간 원").unwrap();
    let mut order: Vec<usize> = (0..iv.len()).collect();
    order.sort_by(|&x, &y| {
        dot(&qv[ko_red_circle], &iv[y])
            .partial_cmp(&dot(&qv[ko_red_circle], &iv[x]))
            .unwrap()
    });
    println!("\n'빨간 원' top-3 (색/도형/점수):");
    for &i in &order[..3] {
        println!(
            "  {} {} {:.3}",
            COLORS[imgs[i].1].1,
            SHAPES[imgs[i].2].2,
            dot(&qv[ko_red_circle], &iv[i])
        );
    }

    // 2) 양자화 모델과의 일치도
    if let Some(alt) = a.get("vision-alt") {
        let v2 = ImageEmbedder::load(Path::new(alt), cfg).expect("alt vision");
        let iv2 = v2.embed_images(&refs).expect("embed");
        report_cos("이미지 인코더 alt vs 기준", &iv, &iv2);
    }
    if let Some(alt) = a.get("text-alt") {
        let t2 = ClipTextEmbedder::load(
            Path::new(alt),
            Path::new(a.require("tokenizer")),
            Path::new(a.require("dense")),
            cfg,
        )
        .expect("alt text");
        let qv2 = t2.embed_texts(&q_text).expect("embed");
        report_cos("CLIP 텍스트 alt vs 기준", &qv, &qv2);
    }

    // 3) e5 의미 검색 (한/영 교차)
    if let Some(m) = a.get("e5") {
        let e5 =
            E5Embedder::load(Path::new(m), Path::new(a.require("e5-tokenizer")), cfg).expect("e5");
        let passages = [
            "회의에서 다음 분기 제품 출시 일정과 예산을 논의했다",
            "A dog is running on the beach at sunset",
            "외장 하드 디스크가 연결되지 않으면 파일이 오프라인으로 표시된다",
            "The chef prepared a traditional Korean dinner with kimchi stew",
            "제주도 해안 도로를 따라 자동차로 여행한 기록 영상",
            "Quarterly financial results exceeded the analysts' expectations",
        ];
        let queries = [
            ("분기 제품 출시 회의", 0),
            ("dog playing at the seaside", 1),
            ("USB 드라이브를 뽑았을 때 파일 상태", 2),
            ("김치찌개 요리", 3),
            ("road trip on Jeju island", 4),
            ("실적 발표 수익", 5),
        ];
        let pv = e5.embed(TextRole::Passage, &passages).expect("passages");
        let qs: Vec<&str> = queries.iter().map(|q| q.0).collect();
        let qv = e5.embed(TextRole::Query, &qs).expect("queries");
        let hits = queries
            .iter()
            .zip(&qv)
            .filter(|((_, want), v)| {
                let best = (0..pv.len())
                    .max_by(|&x, &y| dot(v, &pv[x]).partial_cmp(&dot(v, &pv[y])).unwrap())
                    .unwrap();
                best == *want
            })
            .count();
        println!(
            "\n## e5 의미 검색 (한/영 교차 질의 {} 개, 문서 {} 개): top-1 정확 {hits}/{}",
            queries.len(),
            passages.len(),
            queries.len()
        );
        if let Some(alt) = a.get("e5-alt") {
            let e2 = E5Embedder::load(Path::new(alt), Path::new(a.require("e5-tokenizer")), cfg)
                .expect("e5 alt");
            let pv2 = e2.embed(TextRole::Passage, &passages).expect("p");
            let qv2 = e2.embed(TextRole::Query, &qs).expect("q");
            report_cos("e5 alt vs 기준 (문서)", &pv, &pv2);
            report_cos("e5 alt vs 기준 (질의)", &qv, &qv2);
            let hits2 = queries
                .iter()
                .zip(&qv2)
                .filter(|((_, want), v)| {
                    let best = (0..pv2.len())
                        .max_by(|&x, &y| dot(v, &pv2[x]).partial_cmp(&dot(v, &pv2[y])).unwrap())
                        .unwrap();
                    best == *want
                })
                .count();
            println!("e5 alt top-1 정확 {hits2}/{}", queries.len());
        }
    }
}

fn report_cos(label: &str, a: &[Vec<f32>], b: &[Vec<f32>]) {
    let mut cs: Vec<f32> = a.iter().zip(b).map(|(x, y)| dot(x, y)).collect();
    cs.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let mean = cs.iter().sum::<f32>() / cs.len() as f32;
    println!(
        "{label}: 코사인 평균 {mean:.4} / 최소 {:.4} / P5 {:.4} (n={})",
        cs[0],
        cs[cs.len() / 20],
        cs.len()
    );
}
