//! 임베딩 처리량·지연·RSS 측정 (스레드/배치별로 **프로세스를 나눠** 실행해 RSS 가 섞이지 않게 한다).
//!
//! 이미지: `bench_embed --kind image --model <vision.onnx> --dir <jpg 폴더> --threads 4 --batch 8 --count 200`
//! CLIP 텍스트: `--kind clip-text --model <model.onnx> --tokenizer <tokenizer.json> --dense <dense.safetensors>`
//! e5: `--kind e5 --model <model.onnx> --tokenizer <tokenizer.json> [--text-len short|long]`
//!
//! 이미지는 배치마다 디코딩+전처리(`pre`)와 추론(`infer`)을 따로 잰다. 처음 1 배치는 워밍업으로 제외한다.

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Args, fail, print_conditions, stats, sysinfo};
use mam_ai::local::embed::image_pre::{CLIP_TENSOR_LEN, decode_image, preprocess_clip};
use mam_ai::local::embed::{ClipTextEmbedder, E5Embedder, EmbedConfig, ImageEmbedder, TextRole};

const SHORT: &[&str] = &[
    "빨간 원",
    "a red circle on a white background",
    "바닷가에서 노는 강아지",
    "two people riding bicycles in the city",
    "회의실에서 발표하는 사람",
    "a plate of food on a wooden table",
    "눈 덮인 산과 맑은 하늘",
    "a close-up photo of a cat sleeping",
];

fn long_text() -> String {
    "오늘 회의에서는 다음 분기 제품 출시 일정과 예산 계획을 논의했습니다. 참석자들은 마케팅 일정, 개발 진행 상황, 그리고 \
     외부 협력사와의 계약 조건을 검토했고, 다음 주까지 각자 담당 항목의 세부 계획서를 제출하기로 했습니다. In the meeting we \
     also reviewed the risks of the schedule, the dependency on third-party vendors, and the fallback options if the release \
     has to be postponed by two weeks. Finally the team agreed to hold a short follow-up call every morning until the launch."
        .to_string()
}

fn images_in(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| fail(&format!("read {}: {e}", dir.display())))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png"))
        })
        .collect();
    v.sort();
    v
}

fn main() {
    let a = Args::parse();
    let kind = a.require("kind").to_string();
    let threads: usize = a.num("threads", 2);
    let batch: usize = a.num("batch", 8);
    let count: usize = a.num("count", 64);
    println!("\n== bench_embed kind={kind} threads={threads} batch={batch} count={count} ==");
    let cond = print_conditions();
    let cfg = EmbedConfig {
        threads,
        batch_size: batch,
    };
    let rss0 = sysinfo::current_rss_bytes();
    let t = Instant::now();

    let mut batch_lat: Vec<Duration> = Vec::new();
    let mut pre_lat: Vec<Duration> = Vec::new();
    let (load, total, items, rss_loaded);

    match kind.as_str() {
        "image" => {
            let emb = ImageEmbedder::load(Path::new(a.require("model")), cfg)
                .unwrap_or_else(|e| fail(&e.to_string()));
            load = t.elapsed();
            rss_loaded = sysinfo::current_rss_bytes();
            let files = images_in(Path::new(a.require("dir")));
            if files.is_empty() {
                fail("no images in --dir");
            }
            let mut idx = 0usize;
            let mut next_batch = |n: usize| -> Vec<&PathBuf> {
                (0..n)
                    .map(|_| {
                        let f = &files[idx % files.len()];
                        idx += 1;
                        f
                    })
                    .collect()
            };
            // 워밍업
            let w = next_batch(batch);
            let px: Vec<f32> = w
                .iter()
                .flat_map(|p| preprocess_clip(&decode_image(p).expect("decode")))
                .collect();
            emb.run_pixels(w.len(), px).expect("warmup");

            let started = Instant::now();
            let mut done = 0;
            while done < count {
                let n = batch.min(count - done);
                let paths = next_batch(n);
                let t0 = Instant::now();
                let mut px = Vec::with_capacity(n * CLIP_TENSOR_LEN);
                for p in &paths {
                    px.extend(preprocess_clip(&decode_image(p).expect("decode")));
                }
                let t1 = Instant::now();
                emb.run_pixels(n, px).expect("infer");
                pre_lat.push(t1 - t0);
                batch_lat.push(t1.elapsed());
                done += n;
            }
            total = started.elapsed();
            items = done;
        }
        "clip-text" | "e5" => {
            let long = a.get("text-len") == Some("long");
            let mut texts: Vec<String> = if long {
                vec![long_text()]
            } else {
                SHORT.iter().map(|s| s.to_string()).collect()
            };
            let enc_clip;
            let enc_e5;
            if kind == "clip-text" {
                enc_clip = Some(
                    ClipTextEmbedder::load(
                        Path::new(a.require("model")),
                        Path::new(a.require("tokenizer")),
                        Path::new(a.require("dense")),
                        cfg,
                    )
                    .unwrap_or_else(|e| fail(&e.to_string())),
                );
                enc_e5 = None;
            } else {
                enc_e5 = Some(
                    E5Embedder::load(
                        Path::new(a.require("model")),
                        Path::new(a.require("tokenizer")),
                        cfg,
                    )
                    .unwrap_or_else(|e| fail(&e.to_string())),
                );
                enc_clip = None;
            }
            load = t.elapsed();
            rss_loaded = sysinfo::current_rss_bytes();
            let run = |chunk: &[&str]| {
                if let Some(c) = &enc_clip {
                    c.embed_texts(chunk).expect("embed");
                } else if let Some(e) = &enc_e5 {
                    e.embed(TextRole::Passage, chunk).expect("embed");
                }
            };
            texts.rotate_left(0);
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            run(&refs[..batch.min(refs.len())]); // 워밍업
            let started = Instant::now();
            let mut done = 0;
            let mut i = 0;
            while done < count {
                let n = batch.min(count - done);
                let chunk: Vec<&str> = (0..n).map(|k| refs[(i + k) % refs.len()]).collect();
                i += n;
                let t0 = Instant::now();
                run(&chunk);
                batch_lat.push(t0.elapsed());
                done += n;
            }
            total = started.elapsed();
            items = done;
        }
        other => fail(&format!("--kind must be image|clip-text|e5, got {other}")),
    }

    let s = stats::summarize(&batch_lat);
    let throughput = items as f64 / total.as_secs_f64();
    let peak = sysinfo::peak_rss_bytes();
    println!("\n| metric | value |\n|---|---:|");
    println!("| model load (ms) | {:.0} |", load.as_secs_f64() * 1000.0);
    println!(
        "| RSS before load / after load (MB) | {:.0} / {:.0} |",
        sysinfo::mb(rss0),
        sysinfo::mb(rss_loaded)
    );
    println!("| peak RSS during run (MB) | {:.0} |", sysinfo::mb(peak));
    println!("| batches | {} (batch {batch}) |", s.n);
    println!(
        "| inference per batch P50 / P95 / P99 / max (ms) | {:.1} / {:.1} / {:.1} / {:.1} |",
        s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
    println!(
        "| per-item latency at P50 (ms) | {:.1} |",
        s.p50_ms / batch.min(items.max(1)) as f64
    );
    if !pre_lat.is_empty() {
        let p = stats::summarize(&pre_lat);
        println!(
            "| decode+preprocess per batch P50 / P95 (ms) | {:.1} / {:.1} |",
            p.p50_ms, p.p95_ms
        );
    }
    println!("| throughput (items/s) | {throughput:.2} |");
    println!(
        "JSON: {{\"tool\":\"bench_embed\",\"kind\":\"{kind}\",\"threads\":{threads},\"batch\":{batch},\"items\":{items},\"load_ms\":{:.0},\"rss_loaded_mb\":{:.0},\"peak_rss_mb\":{:.0},\"throughput\":{throughput:.3},\"batch_lat\":{},\"cpu\":{:?}}}",
        load.as_secs_f64() * 1000.0,
        sysinfo::mb(rss_loaded),
        sysinfo::mb(peak),
        s.json(),
        cond.cpu
    );
}
