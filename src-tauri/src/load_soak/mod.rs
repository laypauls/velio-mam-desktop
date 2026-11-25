//! P4 부하·안정성 측정 하네스 (테스트 전용 — `lib.rs` 의 `#[cfg(test)] mod load_soak;`).
//!
//! 비공개 서비스(`SearchService`·`search_hybrid`·AI 어댑터 경계)를 제품과 같은 연결로 쓰기 위해 `src-tauri` 안에 둔다.
//! 측정 테스트는 `#[ignore = "@measurement ..."]` 로 기본 실행에서 빠지고 아래로 실행한다:
//! `cargo test -p mam-desktop --release load_soak -- --ignored --nocapture --test-threads=1`
//!
//! 환경 변수(모두 선택): `MAM_P4_DIR`(기본 `D:\mam-bench\p4-load`), `P4_BASE`(100000), `P4_NEW`(100000),
//! `P4_WINDOW_SECS`(900), `P4_PER_KIND`(300), `P4_M3_ASSETS`(1000000), `P4_M3_VEC_EVERY`(5), `P4_M3_NEW`(20000),
//! `P4_M3_WINDOW_SECS`(300), `P4_M3_PER_KIND`(100), `P4_S1_ROUNDS`(240), `P4_S1_BASE`(3000), `P4_KEEP`(0 = 끝나면 삭제).
//!
//! 범위: 합성 메타데이터·합성 이미지로는 실제 파일시스템·코덱·UI 성능을 판단할 수 없다. 외부 AI·STT·임베딩 인코더는 가짜다.

mod bench_env;
mod bg;
mod fakes;
mod logs;
mod media_gen;
mod metrics;
mod queries;
mod report;
mod scen_conc;
mod scen_load;
mod scen_m12;
mod scen_m3;
mod scen_p4f;
mod scen_s1;
mod stats;
mod synth;

use std::time::Duration;

use report::{bench_root, env_num};

fn finish(root: &std::path::Path) {
    if env_num("P4_KEEP", 0) == 0 {
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
#[ignore = "@measurement: P4 load/soak — cargo test -p mam-desktop --release load_soak -- --ignored --nocapture"]
fn p4_m1_m2_100k() {
    logs::init();
    let root = bench_root().join("m12");
    scen_m12::run(
        &root,
        &scen_m12::Scale {
            base: env_num("P4_BASE", 100_000) as usize,
            new_files: env_num("P4_NEW", 100_000) as usize,
            window: Duration::from_secs(env_num("P4_WINDOW_SECS", 900)),
            per_kind: env_num("P4_PER_KIND", 300) as usize,
        },
        false,
    );
    finish(&root);
}

#[test]
#[ignore = "@measurement: P4 load/soak — cargo test -p mam-desktop --release load_soak -- --ignored --nocapture"]
fn p4_m3_1m() {
    logs::init();
    let root = bench_root().join("m3");
    scen_m3::run(
        &root,
        &scen_m3::M3 {
            assets: env_num("P4_M3_ASSETS", 1_000_000) as usize,
            vec_every: env_num("P4_M3_VEC_EVERY", 5) as usize,
            s: scen_m12::Scale {
                base: 0,
                new_files: env_num("P4_M3_NEW", 20_000) as usize,
                window: Duration::from_secs(env_num("P4_M3_WINDOW_SECS", 300)),
                per_kind: env_num("P4_M3_PER_KIND", 100) as usize,
            },
        },
    );
    finish(&root);
}

#[test]
#[ignore = "@measurement: P4F-2 — cargo test -p mam-desktop --release load_soak::p4f_concurrency -- --ignored --nocapture"]
fn p4f_concurrency() {
    logs::init();
    let root = bench_root().join("conc");
    // 설정 목록: "메타:썸네일:ffmpeg" 쉼표 구분 (기본 2:2:2,4:4:2,6:6:2,4:4:1)
    let spec = std::env::var("P4F_CONFIGS").unwrap_or_else(|_| "2:2:2,4:4:2,6:6:2,4:4:1".into());
    let configs: Vec<scen_conc::Config> = spec
        .split(',')
        .filter_map(|c| {
            let mut it = c.split(':').filter_map(|v| v.parse::<usize>().ok());
            Some(scen_conc::Config {
                meta: it.next()?,
                thumb: it.next()?,
                ffmpeg_threads: it.next()? as u32,
            })
        })
        .collect();
    scen_conc::run(
        &root,
        &scen_conc::Scale {
            base_assets: env_num("P4F_BASE", 20_000) as usize,
            files: env_num("P4F_FILES", 10_000) as usize,
            light: env_num("P4F_LIGHT", 0) != 0,
        },
        &configs,
    );
    finish(&root);
}

#[test]
#[ignore = "@measurement: P4F — cargo test -p mam-desktop --release load_soak::p4f -- --ignored --nocapture"]
fn p4f_allowed_set() {
    logs::init();
    let root = bench_root().join("p4f");
    scen_p4f::run(
        &root,
        env_num("P4F_ASSETS", 1_000_000) as usize,
        env_num("P4F_PER_CASE", 20) as usize,
    );
    finish(&root);
}

#[test]
#[ignore = "@measurement: P4 load/soak — cargo test -p mam-desktop --release load_soak -- --ignored --nocapture"]
fn p4_s1_soak() {
    logs::init();
    let root = bench_root().join("s1");
    scen_s1::run(
        &root,
        &scen_s1::S1 {
            base_files: env_num("P4_S1_BASE", 3_000) as usize,
            rounds: env_num("P4_S1_ROUNDS", 240) as usize,
            add: 25,
            rename: 10,
            delete: 10,
            edits: 10,
        },
        false,
    );
    finish(&root);
}

#[cfg(test)]
mod smoke {
    //! T-p4-smoke-01 — 하네스 배선이 썩지 않게 하는 상시 시험 (수십 파일, 임시 폴더). 분류: Happy / Business rule (원본 보호·정합성) /
    //! External (가짜 외부 AI·STT 경로 통과)
    use std::sync::Arc;
    use std::time::Duration;

    use super::bench_env::{Bench, Options};
    use super::media_gen::{Templates, fingerprint, generate};
    use super::queries::{Ctx, HYBRID, index_docs, run_fixed};
    use super::report::wait_for;
    use super::scen_m12::{build_synth_library, wait_ready};

    #[test]
    fn t_p4_smoke_01_harness_runs_scan_process_ai_and_search_end_to_end() {
        // Arrange: 합성 기준 40건(벡터 포함) + 신규 실제 파일 12개
        let t0 = std::time::Instant::now();
        let dir = tempfile::TempDir::new().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let templates = Templates::build(&root.join("scratch"), true);
        let new_dir = root.join("new");
        generate(&new_dir, &templates, 0..12, 3);
        let before = fingerprint(&new_dir);
        let lib = build_synth_library(&root, 40, 1);
        eprintln!("[smoke] synth library {:?}", t0.elapsed());
        let bench: Arc<Bench> = Bench::start(&root, &Options::default());
        eprintln!("[smoke] started {:?}", t0.elapsed());

        // Act 1: 색인 준비 후 질의 집합 한 바퀴 (하이브리드 포함)
        assert!(
            wait_ready(&bench, 40, Duration::from_secs(60)).is_some(),
            "index was not ready"
        );
        eprintln!("[smoke] index ready {:?}", t0.elapsed());
        let rep = run_fixed(
            &bench.search,
            &Ctx {
                folder_id: lib.folder_id,
            },
            1,
            0,
            1,
        );
        eprintln!("[smoke] queries {:?}", t0.elapsed());
        // Assert 1
        assert_eq!(rep.errors, 0, "{:?}", rep.first_errors);
        assert_eq!(rep.hybrid_total, 1);
        assert_eq!(rep.hybrid_degraded, 0, "hybrid fell back to keyword");
        assert_eq!(rep.samples[HYBRID].len(), 1);

        // Act 2: 신규 폴더 스캔 → 메타·썸네일·해시 → 색인, 이어서 AI 2건 (가짜 외부·STT·합성 임베딩)
        bench.ingest.add_folder(&new_dir.to_string_lossy()).unwrap();
        let processed = wait_for(Duration::from_secs(60), || {
            bench.count("SELECT count(*) FROM media_assets") == 52
                && bench.count(
                    "SELECT count(*) FROM asset_derivatives WHERE kind = 'thumb' AND asset_id > 40 AND state IN ('done','unsupported')",
                ) == 12
                && bench.jobs_busy() == (0, 0)
        });
        assert!(
            processed,
            "pipeline did not drain: jobs={:?}",
            bench.job_counts()
        );
        eprintln!("[smoke] pipeline drained {:?}", t0.elapsed());
        let queued = bench
            .scheduler
            .analyze_assets(&[41, 42], None, false, &bench.rt)
            .unwrap();
        assert!(queued.queued > 0, "no asset was queued for AI");
        let ai_done = wait_for(Duration::from_secs(60), || bench.jobs_busy() == (0, 0));

        // Assert 2: 정합성·오류 0·원본 불변
        eprintln!("[smoke] ai drained {:?}", t0.elapsed());
        assert!(ai_done, "AI jobs did not drain");
        let synced = wait_for(Duration::from_secs(60), || {
            bench.outbox_pending() == 0
                && index_docs(&bench.search) == Some(bench.sqlite_active_docs())
        });
        eprintln!("[smoke] index synced {:?}", t0.elapsed());
        assert!(synced, "index did not converge to SQLite");
        assert_eq!(
            bench.count("SELECT count(*) FROM jobs WHERE status = 'failed'"),
            0
        );
        assert_eq!(
            bench.count("SELECT count(*) FROM search_outbox WHERE status = 'failed'"),
            0
        );
        assert_eq!(
            bench.count("SELECT count(*) FROM ai_task_state WHERE state = 'failed'"),
            0
        );
        assert!(
            bench
                .embed_stats
                .jobs
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0
        );
        assert_eq!(before, fingerprint(&new_dir), "originals must not change");
        bench.shutdown();
        eprintln!("[smoke] shutdown {:?}", t0.elapsed());
    }
}
