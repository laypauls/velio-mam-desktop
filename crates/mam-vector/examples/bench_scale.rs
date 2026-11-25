//! P4 벡터 규모 측정 — 자산 100만(시각 220만 x 512-d + 의미 100만 x 384-d) 제품 경로 측정.
//!
//! 단계를 프로세스로 나눈다 (RSS·열기 시간이 서로 오염되지 않게):
//! - `--phase build`   : 적재(raw f16 + SQLite) -> `Generations::rebuild` -> 크기 보고
//! - `--phase latency` : 새 프로세스에서 view 로 열어 지연 측정 (정답용 f32 행을 올리지 않음)
//! - `--phase recall`  : 같은 합성 벡터를 f32 로 재생성해 전수 비교 정답과 Recall@10 대조
//!
//! 실행: `cargo run --release -p mam-vector --example bench_scale -- --phase build --dir D:\mam-bench\p4-vector`
//! 예행연습: `--scale-div 20` (자산 5만). 합성 클러스터 벡터이므로 지연·크기 전용이다 (관련도에 대해 말하지 않는다).

#[path = "common/mod.rs"]
mod common;
#[path = "common/scale_build.rs"]
mod scale_build;
#[path = "common/scale_data.rs"]
mod scale_data;
#[path = "common/scale_latency.rs"]
mod scale_latency;
#[path = "common/scale_recall.rs"]
mod scale_recall;

use common::sysinfo::{mb, peak_rss_bytes};
use common::{args, print_conditions};
use scale_build::Paths;
use scale_data::Scale;

fn main() {
    let a = args::Args::parse();
    let phase = a.get("phase").unwrap_or("build").to_string();
    let dir = a
        .path("dir")
        .unwrap_or_else(|| args::fail("--dir is required"));
    let sc = Scale::new(a.num("scale-div", 1u64));
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let p = Paths { root: dir };

    println!("== bench_scale phase={phase} ==");
    print_conditions();
    println!(
        "dataset: 자산 {} (이미지 {} + 영상 {} x 5프레임), 시각 {} x 512-d, 의미 {} x 384-d, f16 flat, 스레드 {threads}",
        sc.assets(),
        sc.images,
        sc.videos,
        sc.visual(),
        sc.assets()
    );
    match phase.as_str() {
        "build" => scale_build::build(&p, sc, threads),
        "latency" => {
            let q = a.num("queries", 200usize);
            let o = scale_latency::Opts {
                queries: q,
                sweep_queries: a.num("sweep-queries", 100usize),
                similar: a.num("similar", q),
                sweep_similar: a.num("sweep-similar", 100usize),
                map_runs: a.num("map-runs", 12usize),
                e2e_runs: a.num("e2e-runs", 12usize),
                warmup: a.num("warmup", 10usize),
                threads: a
                    .get("threads")
                    .map(|s| s.split(',').filter_map(|t| t.parse().ok()).collect())
                    .unwrap_or_default(),
            };
            scale_latency::run(&p, sc, &o);
        }
        "recall" => scale_recall::run(
            &p,
            sc,
            a.num("queries", 100usize),
            a.num("similar", 60usize),
            threads,
        ),
        other => args::fail(&format!("unknown --phase {other}")),
    }
    println!("\n(프로세스 최대 RSS {:.0} MB)", mb(peak_rss_bytes()));
}
