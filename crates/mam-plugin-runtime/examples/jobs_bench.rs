//! P5 1M 행 측정 도구 (M1/M2/M3) — 명세 `docs/requests/2026-11/20261122-170245-architect-plugin-runtime-jobs-spec.md` §B9.
//!
//! ```text
//! cargo run --release -p mam-plugin-runtime --example jobs_bench -- m1  <scratch> [assets=1000000]
//! cargo run --release -p mam-plugin-runtime --example jobs_bench -- m2  <scratch> [rows=1000000]
//! cargo run --release -p mam-plugin-runtime --example jobs_bench -- m3a <scratch> [assets=1000000] [events=1000000]
//! cargo run --release -p mam-plugin-runtime --example jobs_bench -- m3b <scratch> <files> gen|noplugin|plugin|feedonly|clean
//! ```
//!
//! 합성 DB 는 실제 마이그레이션을 적용한 뒤 SQL 로 채우며(합성 메타데이터뿐 — 실제 파일시스템·코덱·UI 성능이 아니다), 각 단계 끝에 데이터 폴더를
//! 지운다. `m3b plugin` 은 환경변수 `MAM_PLUGIN_WORKER_EXE`(릴리스 워커)가 필요하다. 결과는 표준 출력 — 파일로 리디렉션해 보관한다.
//! 이 예제는 `#[ignore]` 시험이 아니라 수동 실행 도구다 (CI 에서 돌지 않는다).

#[path = "jobs_bench/env.rs"]
mod env;
#[path = "jobs_bench/m1.rs"]
mod m1;
#[path = "jobs_bench/m2.rs"]
mod m2;
#[path = "jobs_bench/m3a.rs"]
mod m3a;
#[path = "jobs_bench/m3b.rs"]
mod m3b;
#[path = "jobs_bench/synth.rs"]
mod synth;
#[path = "jobs_bench/util.rs"]
mod util;

use std::path::PathBuf;

fn num(args: &[String], i: usize, default: u64) -> u64 {
    args.get(i).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(cmd), Some(scratch)) = (args.first(), args.get(1)) else {
        eprintln!("usage: jobs_bench m1|m2|m3a|m3b <scratch-dir> [sizes…] (see the file header)");
        std::process::exit(2);
    };
    let scratch = PathBuf::from(scratch);
    util::say(format!("jobs_bench {cmd} scratch={} args={:?}", scratch.display(), &args[2..]));
    match cmd.as_str() {
        "m1" => m1::run(&scratch, num(&args, 2, 1_000_000)),
        "m2" => m2::run(&scratch, num(&args, 2, 1_000_000)),
        "m3a" => m3a::run(&scratch, num(&args, 2, 1_000_000), num(&args, 3, 1_000_000)),
        "m3b" => {
            let files = usize::try_from(num(&args, 2, 100_000)).unwrap_or(100_000);
            match args.get(3).map(String::as_str) {
                Some("gen") => m3b::generate(&scratch.join("m3b").join("files"), files),
                Some("noplugin") => m3b::run(&scratch, files, m3b::Mode::NoPlugin),
                Some("plugin") => m3b::run(&scratch, files, m3b::Mode::Plugin),
                Some("feedonly") => m3b::run(&scratch, files, m3b::Mode::FeedOnly),
                Some("clean") => drop(m3b::clean(&scratch)),
                _ => eprintln!("m3b <scratch> <files> gen|noplugin|plugin|feedonly|clean"),
            }
        }
        _ => eprintln!("unknown subcommand {cmd}"),
    }
}
