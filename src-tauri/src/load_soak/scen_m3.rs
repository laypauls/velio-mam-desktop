//! M3 — 100만 합성 자산에서 키워드·하이브리드 지연 (유휴, 그리고 신규 파일 스캔·처리·AI 부하 중).
//!
//! 의미 벡터는 `vec_every` 번째 자산에만 있다(AI 분석을 마친 일부). 100만 전부에 384차원 벡터를 두면 원시 파일만 약 0.8GB(f16),
//! 하이브리드의 활성 맵 읽기도 그만큼 커져 별도 측정 대상이다 — 보고서에 벡터 수를 함께 적는다.

use std::path::Path;
use std::time::{Duration, Instant};

use super::bench_env::{Bench, Options};
use super::media_gen::{Templates, fingerprint, generate};
use super::metrics::{mb, proc_now};
use super::queries::{Ctx, run_fixed};
use super::report::{dir_bytes, gb, print_conditions, print_query_table};
use super::scen_load::{Phase, load_phase};
use super::scen_m12::{Scale, build_synth_library, wait_ready};

pub struct M3 {
    pub assets: usize,
    pub vec_every: usize,
    pub s: Scale,
}

pub fn run(root: &Path, m: &M3) {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root).expect("root");
    print_conditions(
        "M3 조건",
        &[
            (
                "합성 기준 라이브러리",
                format!("{} 자산 (파일 없음, 처리 완료 상태)", m.assets),
            ),
            (
                "의미 벡터",
                format!(
                    "자산 {}개마다 1개 (약 {:.0}%)",
                    m.vec_every,
                    100.0 / m.vec_every as f64
                ),
            ),
            ("부하 단계 신규 실제 파일", format!("{}", m.s.new_files)),
        ],
    );
    let templates = Templates::build(&root.join("scratch"), false);
    let new_dir = root.join("new");
    let made = generate(&new_dir, &templates, 0..m.s.new_files as u64, 13);
    let fp_before = fingerprint(&new_dir);
    println!(
        "- 신규 파일 생성 {} 개 {:.2} GB {:.0}s",
        made.files,
        gb(made.bytes),
        made.secs
    );

    let t = Instant::now();
    let lib = build_synth_library(root, m.assets, m.vec_every);
    println!(
        "- 합성 적재 총 {:.0}s (자산 {:.0}s, 벡터 {} 건 + 인덱스 세대 {:.0}s)",
        t.elapsed().as_secs_f64(),
        lib.seed_secs,
        lib.vectors,
        lib.vector_secs
    );

    let t_boot = Instant::now();
    let bench = Bench::start(root, &Options::default());
    let ready = wait_ready(&bench, m.assets as i64, Duration::from_secs(3600));
    println!(
        "- 기동~색인 준비(Tantivy 전체 재구축 포함): {} / 준비 직후 RSS {:.0} MB, 최대 {:.0} MB",
        ready.map_or("실패".into(), |_| format!(
            "{:.0}s",
            t_boot.elapsed().as_secs_f64()
        )),
        mb(proc_now().rss),
        mb(proc_now().peak_rss)
    );
    if ready.is_none() {
        bench.shutdown();
        return;
    }
    println!(
        "- 디스크: SQLite {:.2} GB, Tantivy {:.2} GB, 벡터 {:.2} GB",
        gb(std::fs::metadata(bench.paths.db()).map_or(0, |m| m.len())),
        gb(dir_bytes(&bench.paths.tantivy())),
        gb(dir_bytes(
            &bench.paths.root().join("indexes").join("vectors")
        ))
    );
    let ctx = Ctx {
        folder_id: lib.folder_id,
    };
    let _ = run_fixed(&bench.search, &ctx, 2, 3, 7);
    let m1 = run_fixed(&bench.search, &ctx, m.s.per_kind, 10, 42);
    print_query_table(&format!("M3/M1 유휴 — {} 자산", m.assets), &m1);
    println!(
        "- 유휴 측정 직후 RSS {:.0} MB (최대 {:.0} MB)",
        mb(proc_now().rss),
        mb(proc_now().peak_rss)
    );

    let label = m.s.new_files.to_string();
    let phase = Phase {
        bench: &bench,
        ctx: &ctx,
        new_dir: &new_dir,
        fp_before,
        floor: m.assets as i64,
        window: m.s.window,
        per_kind: m.s.per_kind,
        label: &label,
    };
    load_phase(&phase, &m1);
    bench.shutdown();
}
