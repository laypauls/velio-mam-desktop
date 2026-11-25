//! M1(유휴 기준선)·M2(스캔·처리·AI 부하 중 검색) — 10만 합성 자산 + 10만 신규 실제 파일.

use std::path::Path;
use std::time::{Duration, Instant};

use mam_db::SystemClock;
use mam_vector::{EmbedStore, Generations};

use super::bench_env::{Bench, Options, prepare_db, register_folder_row};
use super::media_gen::{Templates, fingerprint, generate};
use super::metrics::{mb, proc_now};
use super::queries::{Ctx, KINDS, Report, index_docs, run_fixed};
use super::report::{dir_bytes, gb, print_conditions, print_query_table, wait_for};
use super::scen_load::{Phase, load_phase};
use super::synth::{SeedSpec, seed_assets, seed_semantic_vectors, seed_tags, semantic_key};

pub struct SynthLib {
    pub folder_id: i64,
    pub seed_secs: f64,
    pub vector_secs: f64,
    pub vectors: usize,
}

/// 합성 라이브러리를 만든다: 자산 `n` 건(처리 완료 상태) + 의미 벡터(`vec_every` 번째 자산마다) + 활성 벡터 인덱스 세대.
pub fn build_synth_library(root: &Path, n: usize, vec_every: usize) -> SynthLib {
    let (paths, mut conn) = prepare_db(root);
    conn.pragma_update(None, "synchronous", "OFF")
        .expect("pragma");
    let base_dir = root.join("base");
    let (volume_id, folder_id, rel_root) = register_folder_row(&mut conn, &base_dir);
    seed_tags(&conn).expect("tags");
    let now = mam_db::Clock::now_ms(&SystemClock);
    let t = Instant::now();
    let mut first = 1usize;
    while first <= n {
        let count = (n + 1 - first).min(100_000);
        seed_assets(
            &mut conn,
            &SeedSpec {
                volume_id,
                root_rel: &rel_root,
                first_id: first as i64,
                count,
                seed: 1000 + first as u64,
                now_ms: now,
            },
        )
        .expect("seed assets");
        first += count;
    }
    let seed_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let store = EmbedStore::new(paths.root().join("indexes").join("vectors")).expect("store");
    let vectors = seed_semantic_vectors(
        &mut conn,
        &store,
        (1..=n as i64).filter(|i| (*i as usize).is_multiple_of(vec_every.max(1))),
        now,
    )
    .expect("seed vectors");
    let key = semantic_key();
    let report = Generations::new(&store, &key)
        .and_then(|g| g.rebuild(&conn, &store, &key))
        .expect("rebuild vector generation");
    assert_eq!(report.vectors as usize, vectors);
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .expect("checkpoint");
    SynthLib {
        folder_id,
        seed_secs,
        vector_secs: t.elapsed().as_secs_f64(),
        vectors,
    }
}

/// 색인이 열리고 문서 수가 `expected` 가 되며 Outbox 와 작업이 비워질 때까지 기다린다. 걸린 초 (실패 시 None).
pub fn wait_ready(b: &Bench, expected: i64, timeout: Duration) -> Option<f64> {
    let t = Instant::now();
    let ok = wait_for(timeout, || {
        index_docs(&b.search) == Some(expected)
            && b.outbox_pending() == 0
            && b.jobs_busy() == (0, 0)
    });
    ok.then(|| t.elapsed().as_secs_f64())
}

pub(super) fn derivative_done(b: &Bench, kind: &str, floor: i64) -> i64 {
    b.count(&format!(
        "SELECT count(*) FROM asset_derivatives WHERE kind = '{kind}' AND asset_id > {floor}
         AND state IN ('done', 'unsupported')"
    ))
}

pub(super) fn counts_line(b: &Bench) -> String {
    b.job_counts()
        .iter()
        .filter(|(_, c)| c.pending + c.running + c.failed > 0)
        .map(|(k, c)| format!("{}:{}/{}/{}", k.as_str(), c.pending, c.running, c.failed))
        .collect::<Vec<_>>()
        .join(" ")
}

/// M1 대비 증가율 표.
pub fn print_compare(base_label: &str, base: &Report, other_label: &str, other: &Report) {
    println!("\n### 증가율: {other_label} vs {base_label} (P95 기준)");
    println!(
        "| 질의 | {base_label} P95 ms | {other_label} P95 ms | 배율 | {base_label} P50 | {other_label} P50 |\n|---|---:|---:|---:|---:|---:|"
    );
    for (k, name) in KINDS.iter().enumerate() {
        let (a, b) = (base.summary(k), other.summary(k));
        let ratio = if a.p95 > 0.0 { b.p95 / a.p95 } else { 0.0 };
        println!(
            "| {name} | {:.2} | {:.2} | {ratio:.2}x | {:.2} | {:.2} |",
            a.p95, b.p95, a.p50, b.p50
        );
    }
    let (a, b) = (base.all(), other.all());
    println!(
        "| **전체** | {:.2} | {:.2} | {:.2}x | {:.2} | {:.2} |",
        a.p95,
        b.p95,
        if a.p95 > 0.0 { b.p95 / a.p95 } else { 0.0 },
        a.p50,
        b.p50
    );
}

pub struct Scale {
    pub base: usize,
    pub new_files: usize,
    pub window: Duration,
    pub per_kind: usize,
}

/// M1 + M2 + 사후 유휴 측정. `light` 면 템플릿을 작게(스모크) 만든다.
pub fn run(root: &Path, s: &Scale, light: bool) {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root).expect("root");
    print_conditions(
        "M1/M2 조건",
        &[
            (
                "합성 기준 라이브러리",
                format!(
                    "{} 자산 (파일 없음, 처리 완료 상태, 의미 벡터 100%)",
                    s.base
                ),
            ),
            (
                "신규 실제 파일",
                format!("{} (JPEG 88% / PNG 4% / mp4 4% / WAV 4%)", s.new_files),
            ),
            ("부하 창", format!("{} s 상한", s.window.as_secs())),
        ],
    );

    // 1) 신규 실제 파일 생성 + 원본 지문
    let templates = Templates::build(&root.join("scratch"), light);
    let new_dir = root.join("new");
    let made = generate(&new_dir, &templates, 0..s.new_files as u64, 11);
    let fp_before = fingerprint(&new_dir);
    println!(
        "- 신규 파일 생성: {} 개, {:.2} GB(내용), {:.0}s, 영상 {} / 음성 {} (ffmpeg 템플릿 {})",
        made.files,
        gb(made.bytes),
        made.secs,
        made.videos,
        made.audios,
        if templates.has_video() {
            "있음"
        } else {
            "없음 — 이미지/WAV 만"
        }
    );

    // 2) 합성 기준 라이브러리
    let lib = build_synth_library(root, s.base, 1);
    println!(
        "- 합성 적재: 자산 {:.0}s, 벡터 {} 건 + 인덱스 세대 {:.0}s",
        lib.seed_secs, lib.vectors, lib.vector_secs
    );

    // 3) 서비스 기동 (색인은 SQLite 에서 재구축)
    let t_boot = Instant::now();
    let bench = Bench::start(root, &Options::default());
    let ready = wait_ready(&bench, s.base as i64, Duration::from_secs(1800));
    println!(
        "- 기동~색인 준비(재구축 포함): {} / 최초 유휴 RSS {:.0} MB",
        ready.map_or("실패".into(), |_| format!(
            "{:.1}s",
            t_boot.elapsed().as_secs_f64()
        )),
        mb(proc_now().rss)
    );
    if ready.is_none() {
        println!(
            "준비 실패: docs={:?} outbox={} jobs={:?}",
            index_docs(&bench.search),
            bench.outbox_pending(),
            bench.jobs_busy()
        );
        bench.shutdown();
        return;
    }
    let ctx = Ctx {
        folder_id: lib.folder_id,
    };
    let docs_dir = bench.paths.tantivy();
    println!(
        "- 디스크: SQLite {:.2} GB, Tantivy {:.2} GB, 벡터 {:.2} GB",
        gb(std::fs::metadata(bench.paths.db()).map_or(0, |m| m.len())),
        gb(dir_bytes(&docs_dir)),
        gb(dir_bytes(
            &bench.paths.root().join("indexes").join("vectors")
        ))
    );

    // 4) M1: 유휴 기준선
    let _ = run_fixed(&bench.search, &ctx, 3, 5, 7); // 캐시 워밍
    let m1 = run_fixed(&bench.search, &ctx, s.per_kind, 20, 42);
    let idle = proc_now();
    print_query_table(
        &format!(
            "M1 유휴 기준선 — {} 자산, 백그라운드 작업 없음, 색인 안정",
            s.base
        ),
        &m1,
    );
    println!(
        "- M1 직후 RSS {:.0} MB, 핸들 {}, 스레드 {}",
        mb(idle.rss),
        idle.handles,
        idle.threads
    );

    // 5~8) 부하 단계 (스캔·처리·AI 와 동시에 검색) + 사후 유휴 + 정합성
    let label = s.new_files.to_string();
    let phase = Phase {
        bench: &bench,
        ctx: &ctx,
        new_dir: &new_dir,
        fp_before,
        floor: s.base as i64,
        window: s.window,
        per_kind: s.per_kind,
        label: &label,
    };
    load_phase(&phase, &m1);
    bench.shutdown();
}
