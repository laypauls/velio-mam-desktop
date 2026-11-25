//! 실제 CLIP 임베딩 검색 품질 (COCO val2017 5,000장) - `embed_dataset` 출력이 입력이다.
//!
//! `eval_coco --vecs <embed_dataset out> [--text <onnx> --tokenizer <json> --dense <safetensors>]`
//! `--text` 를 주면 캡션·카테고리 질의 벡터를 그 모델로 다시 만든다 (int8 텍스트 모델 비교용). 없으면 덤프 벡터를 쓴다.
//!
//! - 영어 캡션(이미지당 1개) -> 이미지 Recall@1/5/10 (5,000 질의 x 5,000 이미지)
//! - 카테고리명 한/영 -> 이미지: 해당 객체가 면적 10% 이상인 이미지가 관련 (정밀도@10, top-1 적중, 10건 안 적중)
//!
//! COCO 는 영어 캡션이라 한국어는 카테고리 질의만 평가한다 (한국어 문장 질의 평가는 미수행).

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

use common::{Args, fail};
use mam_ai::local::embed::{ClipTextEmbedder, EmbedConfig};

const DIM: usize = 512;

fn read_f32(path: &Path) -> Vec<Vec<f32>> {
    let b = std::fs::read(path).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
    b.chunks_exact(DIM * 4)
        .map(|r| {
            r.chunks_exact(4)
                .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                .collect()
        })
        .collect()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn top_k(q: &[f32], rows: &[Vec<f32>], k: usize) -> Vec<usize> {
    let mut s: Vec<(f32, usize)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (dot(q, r), i))
        .collect();
    s.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    s.into_iter().take(k).map(|(_, i)| i).collect()
}

fn main() {
    let a = Args::parse();
    let dir = PathBuf::from(a.require("vecs"));
    let meta = std::fs::read_to_string(dir.join("image_meta.tsv")).expect("meta");
    let all = read_f32(&dir.join("image_vectors.f32"));
    // 원본 뷰만 (행 순서 = 이미지 순서)
    let orig_rows: Vec<usize> = meta
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("orig\t"))
        .map(|(i, _)| i)
        .collect();
    let images: Vec<Vec<f32>> = orig_rows.iter().map(|&i| all[i].clone()).collect();
    println!("COCO val2017 images (orig views): {}", images.len());

    let caps_meta = std::fs::read_to_string(dir.join("caption_meta.tsv")).expect("caps");
    let rel_tsv = std::fs::read_to_string(dir.join("class_relevant.tsv")).expect("rel");
    let classes: Vec<(String, String, Vec<usize>)> = rel_tsv
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.splitn(4, '\t').collect();
            let rel = f[3]
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            (f[1].to_string(), f[2].to_string(), rel)
        })
        .collect();

    let (cap_vecs, en_vecs, ko_vecs) = if let Some(text) = a.get("text") {
        let t = ClipTextEmbedder::load(
            Path::new(text),
            Path::new(a.require("tokenizer")),
            Path::new(a.require("dense")),
            EmbedConfig {
                threads: a.num("threads", 6),
                batch_size: 16,
            },
        )
        .unwrap_or_else(|e| fail(&e.to_string()));
        let caps: Vec<&str> = caps_meta
            .lines()
            .map(|l| l.split_once('\t').unwrap().1)
            .collect();
        let en: Vec<String> = classes
            .iter()
            .map(|c| format!("a photo of a {}", c.0))
            .collect();
        let ko: Vec<String> = classes.iter().map(|c| format!("{} 사진", c.1)).collect();
        let r = |v: &[String]| {
            t.embed_texts(&v.iter().map(String::as_str).collect::<Vec<_>>())
                .expect("embed")
        };
        (t.embed_texts(&caps).expect("caps"), r(&en), r(&ko))
    } else {
        let rd = |n: &str| {
            let b = std::fs::read(dir.join(n)).expect("vec file");
            b.chunks_exact(DIM * 4)
                .map(|r| {
                    r.chunks_exact(4)
                        .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
                        .collect::<Vec<f32>>()
                })
                .collect::<Vec<_>>()
        };
        (
            rd("caption_vectors.f32"),
            rd("class_vectors_en.f32"),
            rd("class_vectors_ko.f32"),
        )
    };

    // 캡션 -> 이미지
    let (mut r1, mut r5, mut r10) = (0, 0, 0);
    for (i, q) in cap_vecs.iter().enumerate() {
        let top = top_k(q, &images, 10);
        r1 += (top[0] == i) as usize;
        r5 += top[..5].contains(&i) as usize;
        r10 += top.contains(&i) as usize;
    }
    let n = cap_vecs.len() as f64;
    println!(
        "\n| 영어 캡션 -> 이미지 ({} 질의) | Recall@1 | Recall@5 | Recall@10 |\n|---|---:|---:|---:|",
        cap_vecs.len()
    );
    println!(
        "| COCO val2017 5k | {:.3} | {:.3} | {:.3} |",
        r1 as f64 / n,
        r5 as f64 / n,
        r10 as f64 / n
    );

    // 카테고리 질의
    println!(
        "\n| 카테고리 질의 | 평가 클래스 수 | 정밀도@10 | top-1 적중 | 10건 안 적중 |\n|---|---:|---:|---:|---:|"
    );
    for (label, vecs) in [
        ("영어 `a photo of a X`", &en_vecs),
        ("한국어 `X 사진`", &ko_vecs),
    ] {
        let (mut used, mut p10, mut hit1, mut hit10) = (0, 0.0, 0, 0);
        for (c, q) in classes.iter().zip(vecs.iter()) {
            if c.2.len() < 5 {
                continue; // 관련 이미지가 5장 미만인 클래스는 제외
            }
            used += 1;
            let top = top_k(q, &images, 10);
            let hits = top.iter().filter(|i| c.2.contains(i)).count();
            p10 += hits as f64 / 10.0;
            hit1 += c.2.contains(&top[0]) as usize;
            hit10 += (hits > 0) as usize;
        }
        println!(
            "| {label} | {used} | {:.3} | {:.3} | {:.3} |",
            p10 / used as f64,
            hit1 as f64 / used as f64,
            hit10 as f64 / used as f64
        );
    }
}
