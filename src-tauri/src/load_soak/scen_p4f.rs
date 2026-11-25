//! P4 후속 1 — 하이브리드 허용 집합: 예전 방식(`filtered_asset_ids` 전체 수집, 캐시 없음) 대 새 방식(빠른 수집 + 짧은 캐시).
//!
//! 같은 프로세스·같은 데이터에서 두 방식을 번갈아 돌려 OS 파일 캐시·기계 부하 차이를 줄인다. 의미 벡터는 모든 자산에 있다.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

use mam_core::query::{MediaKind, SearchRequest};

use super::bench_env::{Bench, Options};
use super::metrics::{mb, proc_now};
use super::queries::{Ctx, HYBRID, build};
use super::report::{dir_bytes, gb, print_conditions};
use super::scen_m12::{build_synth_library, wait_ready};
use super::stats::{Lcg, summarize};

const ROUNDS: usize = 4;

fn hybrid_request(rng: &mut Lcg, ctx: &Ctx, tweak: &dyn Fn(&mut SearchRequest)) -> SearchRequest {
    let mut r = build(HYBRID, rng, ctx);
    tweak(&mut r);
    r
}

fn timed(b: &Bench, r: &SearchRequest) -> f64 {
    let t = Instant::now();
    let page = b.search.search(r).expect("hybrid search");
    assert!(page.degraded.is_none(), "hybrid fell back to keyword");
    t.elapsed().as_secs_f64() * 1e3
}

pub fn run(root: &Path, assets: usize, per_case: usize) {
    // P4F_REUSE=1 이고 이전 적재(P4_KEEP=1)가 있으면 합성 라이브러리를 다시 만들지 않는다
    let reuse = super::report::env_num("P4F_REUSE", 0) != 0 && root.join("MAMData").exists();
    if !reuse {
        let _ = std::fs::remove_dir_all(root);
    }
    std::fs::create_dir_all(root).expect("root");
    print_conditions(
        "P4F-1 조건",
        &[
            ("합성 라이브러리", format!("{assets} 자산, 의미 벡터 전부")),
            (
                "케이스당 질의",
                format!("{per_case} (예전/새 방식 번갈아 {ROUNDS}라운드)"),
            ),
        ],
    );
    let folder_id = if reuse {
        println!("- 이전 합성 적재를 재사용");
        None
    } else {
        let lib = build_synth_library(root, assets, 1);
        println!(
            "- 합성 적재: 자산 {:.0}s, 벡터 {} 건 + 인덱스 세대 {:.0}s",
            lib.seed_secs, lib.vectors, lib.vector_secs
        );
        Some(lib.folder_id)
    };
    let t_boot = Instant::now();
    let bench = Bench::start(root, &Options::default());
    let ready = wait_ready(&bench, assets as i64, std::time::Duration::from_secs(3600));
    println!(
        "- 기동~색인 준비 {:?}s, 디스크 SQLite {:.2} GB / Tantivy {:.2} GB / 벡터 {:.2} GB",
        ready.map(|_| t_boot.elapsed().as_secs()),
        gb(std::fs::metadata(bench.paths.db()).map_or(0, |m| m.len())),
        gb(dir_bytes(&bench.paths.tantivy())),
        gb(dir_bytes(
            &bench.paths.root().join("indexes").join("vectors")
        ))
    );
    if ready.is_none() {
        bench.shutdown();
        return;
    }
    let ctx = Ctx {
        folder_id: folder_id
            .unwrap_or_else(|| bench.count("SELECT min(folder_id) FROM managed_folders")),
    };
    let mut rng = Lcg(99);

    // 첫 하이브리드 질의 (맵 워밍업 전) — 예전 방식
    let v = &bench.search.vectors;
    v.legacy_allowed.store(true, Ordering::Relaxed);
    let first_cold = timed(&bench, &hybrid_request(&mut rng, &ctx, &|_| {}));
    println!("- 첫 하이브리드 질의(맵 로드 포함, 예전 방식): {first_cold:.0} ms");
    v.legacy_allowed.store(false, Ordering::Relaxed);

    // 허용 자산 집합 구성 단계별 시간 (기본 필터, 색인 질의만 - 같은 색인·같은 프로세스)
    let filters = bench
        .search
        .filters(&{
            let mut r = hybrid_request(&mut rng, &ctx, &|_| {});
            r.filters.include_missing = false;
            r.filters
        })
        .expect("filters");
    let ms = |f: &dyn Fn() -> usize| {
        let t = Instant::now();
        let n = f();
        (t.elapsed().as_secs_f64() * 1e3, n)
    };
    let h = &bench.search.handle;
    println!(
        "
허용 자산 집합 구성 단계 (ms, 기본 필터, 3회: 이전 수집기 / 새 수집기 / 새 수집기+비트맵)"
    );
    for _ in 0..3 {
        let a = ms(&|| h.filtered_asset_ids_docset(&filters).map_or(0, |v| v.len()));
        let b = ms(&|| h.filtered_asset_ids(&filters).map_or(0, |v| v.len()));
        let c = ms(&|| {
            let ids = h.filtered_asset_ids(&filters).unwrap_or_default();
            mam_vector::IdSet::from_ids(&ids).len()
        });
        println!("- {:.0} ({}건) / {:.0} / {:.0}", a.0, a.1, b.0, c.0);
    }

    type Tweak = Box<dyn Fn(&mut SearchRequest)>;
    let cases: Vec<(&str, Tweak)> = vec![
        (
            "기본 필터 (오프라인 포함, 파일 없음 제외)",
            Box::new(|_| {}),
        ),
        ("온라인만", Box::new(|r| r.filters.include_offline = false)),
        (
            "동영상만",
            Box::new(|r| r.filters.media_types = vec![MediaKind::Video]),
        ),
        (
            "동영상+크기+촬영일",
            Box::new(|r| {
                r.filters.media_types = vec![MediaKind::Video];
                r.filters.size_min = Some(1_000_000);
                r.filters.captured_from = Some(1_609_459_200_000);
                r.filters.captured_to = Some(1_735_689_600_000);
            }),
        ),
        ("태그 1개", Box::new(|r| r.filters.tag_ids = vec![1])),
    ];
    println!(
        "\n| 케이스 | 방식 | n | P50 ms | P95 ms | P99 ms | max ms |\n|---|---|---|---|---|---|---|"
    );
    for (name, tweak) in &cases {
        let (mut legacy, mut miss, mut hit) = (Vec::new(), Vec::new(), Vec::new());
        for _round in 0..ROUNDS {
            v.legacy_allowed.store(true, Ordering::Relaxed);
            for _ in 0..per_case / ROUNDS {
                legacy.push(timed(
                    &bench,
                    &hybrid_request(&mut rng, &ctx, tweak.as_ref()),
                ));
            }
            v.legacy_allowed.store(false, Ordering::Relaxed);
            // 새 방식: 서로 다른 필터 5개를 거쳐 캐시 항목(4)을 밀어낸 뒤, 필터가 바뀐 첫 질의(구성)와 그 뒤 질의(적중)를 나눠 잰다
            for k in 0..5i64 {
                let f = Box::new(move |r: &mut SearchRequest| r.filters.size_max = Some(100 + k));
                timed(&bench, &hybrid_request(&mut rng, &ctx, &*f));
            }
            miss.push(timed(
                &bench,
                &hybrid_request(&mut rng, &ctx, tweak.as_ref()),
            ));
            for _ in 0..per_case / ROUNDS {
                hit.push(timed(
                    &bench,
                    &hybrid_request(&mut rng, &ctx, tweak.as_ref()),
                ));
            }
        }
        for (label, s) in [
            ("예전", &legacy),
            ("새(구성 1회)", &miss),
            ("새(적중)", &hit),
        ] {
            let m = summarize(s);
            println!(
                "| {name} | {label} | {} | {:.0} | {:.0} | {:.0} | {:.0} |",
                m.n, m.p50, m.p95, m.p99, m.max
            );
        }
    }
    println!(
        "- RSS {:.0} MB (최대 {:.0} MB)",
        mb(proc_now().rss),
        mb(proc_now().peak_rss)
    );
    bench.shutdown();
}
