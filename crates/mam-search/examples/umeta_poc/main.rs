//! 사용자 정의 메타데이터 필드 D2(Tantivy JSON projection) PoC — 프로덕션 스키마·조정기는 건드리지 않는다.
//!
//! 실행: `cargo run --release -p mam-search --example umeta_poc -- --phase build|query|sqlite|types [--n 1000000]`
//! 옵션: `--out <dir>`(기본 D:\mam-desktop\target\bench-umeta), `--threads 2 --heap-mb 200`(색인 쓰기, 제품 기본값),
//!       `--sortable`(정렬 i64 대안 경로 `n{id}` 추가 색인, 기본 꺼짐), `--reps 10`, `--no-verify`, `--update 10000`, `--no-umeta`(기준선: 사용자 필드 없는 색인을 index-base 에 구축)
//! 계획 문서: docs/requests/2026-11/20261115-135644-custom-metadata-fields.md (단계 1)

#[path = "../common/args.rs"]
#[allow(dead_code)]
mod args;
#[path = "../common/rng.rs"]
#[allow(dead_code)]
mod rng;
#[path = "../common/stats.rs"]
#[allow(dead_code)]
mod stats;
#[path = "../common/sysinfo.rs"]
#[allow(dead_code)]
mod sysinfo;
#[path = "../common/vocab.rs"]
#[allow(dead_code)]
mod vocab;

mod data;
mod q;
mod sqlite_part;
mod tv_build;
mod tv_query;
mod types;

use std::path::PathBuf;

fn main() {
    let a = args::Args::parse();
    let n: u64 = a.num("n", 1_000_000);
    let out: PathBuf = a
        .path("out")
        .unwrap_or_else(|| PathBuf::from(r"D:\mam-desktop\target\bench-umeta"));
    let phase = a.get("phase").unwrap_or("all").to_string();
    let c = sysinfo::conditions();
    println!(
        "CPU: {}\nRAM: {} GB\nOS: {}\nDisks: {}\nPower: {}\nn={n} out={}",
        c.cpu,
        c.ram_gb,
        c.os,
        c.disks,
        c.power_plan,
        out.display()
    );
    std::fs::create_dir_all(&out).unwrap();
    let all = phase == "all";
    if all || phase == "types" {
        println!("\n## types");
        types::run();
    }
    if all || phase == "build" {
        println!("\n## build");
        let base = a.flag("no-umeta");
        tv_build::build(
            &out.join(if base { "index-base" } else { "index" }),
            n,
            a.num("threads", 2),
            a.num("heap-mb", 200),
            a.flag("sortable"),
            !base,
        );
    }
    if all || phase == "query" {
        println!("\n## query");
        tv_query::run(
            &out.join("index"),
            n,
            a.num("reps", 10),
            !a.flag("no-verify"),
            a.num("update", 10_000),
        );
    }
    if all || phase == "sqlite" {
        println!("\n## sqlite");
        let db = out.join("umeta.db");
        if !a.flag("skip-load") {
            sqlite_part::load(&db, n);
        }
        sqlite_part::measure(&db, n);
    }
    println!("peak RSS {:.0} MB", sysinfo::mb(sysinfo::peak_rss_bytes()));
}
