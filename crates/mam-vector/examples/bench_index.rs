//! 인덱스 구축·열기·질의 측정 (Recall@10/100, P50/P95/P99, RSS, 크기).
//!
//! 구축:  `bench_index --dir <dataset> --phase build --engine flat --quant f16 [--threads N]`
//! 질의:  `bench_index --dir <dataset> --phase query --engine flat --quant f16 --mode load|view --ef 32,64,128,256,512`
//! 둘 다: `--phase both`. 다중 프레임: `--multiframe` (자산 50개를 채우는 k, 자산 단위 Recall@10).
//!
//! 구축 -> 질의는 **프로세스를 나눠** 실행한다 (구축 RSS 와 열기 RSS 가 섞이지 않게).
//! 질의 클라이언트는 단일 스레드이며 전수 비교만 내부에서 `--scan-threads`(기본 전체) 로 병렬화한다.
//! 첫 질의는 열기 직후 1회를 별도로 기록한다 (OS 페이지 캐시는 비우지 못한다 — 32GB RAM 이라 캐시 hit 가 유리).

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::data::{self, Info, Layout};
use common::{args, stats, sysinfo};
use mam_vector::{
    AnnIndex, Engine, FlatIndex, IndexSpec, OpenMode, SearchParams, add_parallel, create_index,
    open_index, save_index, search_assets,
};

fn main() {
    let a = args::Args::parse();
    let l = Layout::new(
        a.path("dir")
            .unwrap_or_else(|| args::fail("--dir is required")),
    );
    let info = Info::read(&l);
    let spec = common::spec_from_args(&a, info.dim);
    let phase = a.get("phase").unwrap_or("both").to_string();
    let name = common::index_name(&spec);
    let dir = l.index(&name);
    println!("\n== bench_index {name} phase={phase} ==");
    let cond = common::print_conditions();
    println!(
        "Dataset: assets={} vectors={} dim={} kind={} clusters={} queries={}",
        info.assets, info.vectors, info.dim, info.kind, info.clusters, info.queries
    );
    let mut json = vec![
        format!("\"tool\":\"bench_index\""),
        format!("\"name\":{}", common::json_str(&name)),
        format!("\"vectors\":{}", info.vectors),
        format!("\"dim\":{}", info.dim),
    ];

    if phase == "build" || phase == "both" {
        build(&a, &l, &info, &spec, &dir, &mut json);
    }
    if phase == "query" || phase == "both" {
        query(&a, &l, &info, &spec, &dir, &mut json);
    }
    json.push(format!("\"conditions\":{}", common::conditions_json(&cond)));
    println!("JSON: {{{}}}", json.join(","));
}

fn build(
    a: &args::Args,
    l: &Layout,
    info: &Info,
    spec: &IndexSpec,
    dir: &std::path::Path,
    json: &mut Vec<String>,
) {
    let threads: usize = a.num(
        "threads",
        std::thread::available_parallelism().map_or(4, |n| n.get()),
    );
    let _ = std::fs::remove_dir_all(dir);
    let rss0 = sysinfo::current_rss_bytes();
    let idx =
        create_index(spec, info.vectors).unwrap_or_else(|e| args::fail(&format!("create: {e}")));
    let t = Instant::now();
    let mut done = 0usize;
    let progress = Instant::now();
    data::for_each_chunk(l, info, 32_768, |ids, rows| {
        add_parallel(&*idx, ids, rows, threads)
            .unwrap_or_else(|e| args::fail(&format!("add: {e}")));
        done += ids.len();
        if done % (32_768 * 8) < 32_768 {
            eprintln!(
                "  added {done}/{} ({:.0}s)",
                info.vectors,
                progress.elapsed().as_secs_f64()
            );
        }
    });
    let build_secs = t.elapsed().as_secs_f64();
    let build_peak = sysinfo::peak_rss_bytes();
    let build_rss = sysinfo::current_rss_bytes();
    let t = Instant::now();
    save_index(dir, &*idx, spec).unwrap_or_else(|e| args::fail(&format!("save: {e}")));
    let save_secs = t.elapsed().as_secs_f64();
    let size = data::dir_size(dir);
    println!("\n| build | value |\n|---|---:|");
    println!("| threads | {threads} |");
    println!("| build seconds | {build_secs:.1} |");
    println!(
        "| throughput (vectors/s) | {:.0} |",
        info.vectors as f64 / build_secs
    );
    println!("| save seconds | {save_secs:.1} |");
    println!("| index size on disk (MB) | {:.1} |", sysinfo::mb(size));
    println!(
        "| engine-reported memory (MB) | {:.1} |",
        sysinfo::mb(idx.memory_bytes() as u64)
    );
    println!(
        "| working set after build (MB, baseline {:.0}) | {:.1} |",
        sysinfo::mb(rss0),
        sysinfo::mb(build_rss)
    );
    println!(
        "| peak working set during build+save (MB) | {:.1} |",
        sysinfo::mb(build_peak)
    );
    json.push(format!(
        "\"build\":{{\"threads\":{threads},\"build_secs\":{build_secs:.1},\"save_secs\":{save_secs:.1},\"size_mb\":{:.1},\"ws_mb\":{:.1},\"peak_ws_mb\":{:.1}}}",
        sysinfo::mb(size), sysinfo::mb(build_rss), sysinfo::mb(build_peak)
    ));
}

fn ids_of(hits: &[mam_vector::Hit]) -> Vec<u64> {
    hits.iter().map(|h| h.id).collect()
}

fn query(
    a: &args::Args,
    l: &Layout,
    info: &Info,
    spec: &IndexSpec,
    dir: &std::path::Path,
    json: &mut Vec<String>,
) {
    let mode = match a.get("mode").unwrap_or("load") {
        "load" => OpenMode::Load,
        "view" => OpenMode::View,
        o => args::fail(&format!("--mode must be load|view, got {o}")),
    };
    let queries = data::read_queries(l, info);
    let truth = data::read_truth(l, info);
    let efs: Vec<usize> = a
        .get("ef")
        .unwrap_or("64")
        .split(',')
        .map(|s| {
            s.parse()
                .unwrap_or_else(|_| args::fail("--ef: comma separated integers"))
        })
        .collect();
    let scan_threads: usize = a.num(
        "scan-threads",
        std::thread::available_parallelism().map_or(4, |n| n.get()),
    );
    let n100: usize = a.num("n100", 300).min(queries.len());
    let n500: usize = a.num("n500", 100).min(queries.len());

    let rss_before = sysinfo::current_rss_bytes();
    if spec.engine == Engine::Flat {
        // 전수 비교는 스레드 수를 구체 타입에서만 바꿀 수 있어 FlatIndex 를 직접 연다 (메타 대조는 save/open 테스트가 다룬다)
        let t = Instant::now();
        let flat = FlatIndex::open(
            &Engine::Flat.data_path(dir),
            info.dim,
            spec.quantization,
            mode,
        )
        .unwrap_or_else(|e| args::fail(&format!("open: {e}")));
        let open_secs = t.elapsed().as_secs_f64();
        let rss_open = sysinfo::current_rss_bytes();
        flat.set_threads(scan_threads);
        run_queries(
            a,
            &flat,
            &queries,
            &truth,
            &efs,
            n100,
            n500,
            info,
            json,
            open_secs,
            rss_before,
            rss_open,
            spec,
            mode,
            scan_threads,
        );
        multiframe(a, &flat, l, &queries, &truth, info, json);
    } else {
        let t = Instant::now();
        let idx = open_index(dir, spec, mode).unwrap_or_else(|e| args::fail(&format!("open: {e}")));
        let open_secs = t.elapsed().as_secs_f64();
        let rss_open = sysinfo::current_rss_bytes();
        run_queries(
            a,
            &*idx,
            &queries,
            &truth,
            &efs,
            n100,
            n500,
            info,
            json,
            open_secs,
            rss_before,
            rss_open,
            spec,
            mode,
            scan_threads,
        );
        multiframe(a, &*idx, l, &queries, &truth, info, json);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_queries(
    _a: &args::Args,
    idx: &dyn AnnIndex,
    queries: &[Vec<f32>],
    truth: &[Vec<u32>],
    efs: &[usize],
    n100: usize,
    n500: usize,
    info: &Info,
    json: &mut Vec<String>,
    open_secs: f64,
    rss_before: u64,
    rss_open: u64,
    spec: &IndexSpec,
    mode: OpenMode,
    scan_threads: usize,
) {
    // 첫 질의 (열기 직후, 같은 프로세스)
    let t = Instant::now();
    let _ = idx
        .search(&queries[0], 10, &SearchParams { ef_search: efs[0] }, None)
        .unwrap();
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("\n| open | value |\n|---|---:|");
    println!("| mode | {mode:?} |");
    println!("| open seconds | {open_secs:.3} |");
    println!("| first query after open (ms) | {first_ms:.1} |");
    println!(
        "| working set before open (MB) | {:.1} |",
        sysinfo::mb(rss_before)
    );
    println!(
        "| working set right after open (MB) | {:.1} |",
        sysinfo::mb(rss_open)
    );
    if spec.engine == Engine::Flat {
        println!("| scan threads | {scan_threads} |");
    }
    println!(
        "\n| ef_search | k | queries | P50 ms | P95 ms | P99 ms | Recall@10 | Recall@100 | k=500 P50 ms | k=500 P95 ms |\n|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    let mut rows = Vec::new();
    for &ef in efs {
        let p = SearchParams { ef_search: ef };
        // k=10: 전체 질의
        let (mut lat, mut r10) = (Vec::new(), 0.0);
        for (q, t) in queries.iter().zip(truth) {
            let s = Instant::now();
            let hits = idx.search(q, 10, &p, None).unwrap();
            lat.push(s.elapsed());
            r10 += data::recall_at(&ids_of(&hits), t, 10);
        }
        let s10 = stats::summarize(&lat);
        // k=100: Recall@100
        let mut r100 = 0.0;
        for (q, t) in queries.iter().zip(truth).take(n100) {
            let hits = idx.search(q, 100, &p, None).unwrap();
            r100 += data::recall_at(&ids_of(&hits), t, 100);
        }
        // k=500: 하이브리드 후보 질의 지연 (측정만)
        let mut lat500: Vec<Duration> = Vec::new();
        for q in queries.iter().take(n500) {
            let s = Instant::now();
            let _ = idx.search(q, 500, &p, None).unwrap();
            lat500.push(s.elapsed());
        }
        let s500 = stats::summarize(&lat500);
        let (rec10, rec100) = (r10 / queries.len() as f64, r100 / n100.max(1) as f64);
        println!(
            "| {ef} | 10 | {} | {:.2} | {:.2} | {:.2} | {rec10:.4} | {rec100:.4} | {:.2} | {:.2} |",
            s10.n, s10.p50_ms, s10.p95_ms, s10.p99_ms, s500.p50_ms, s500.p95_ms
        );
        rows.push(format!(
            "{{\"ef\":{ef},\"k10\":{},\"recall10\":{rec10:.4},\"recall100\":{rec100:.4},\"k500\":{}}}",
            s10.json(),
            s500.json()
        ));
        if spec.engine == Engine::Flat {
            break; // 전수 비교는 ef 가 무의미 — 한 번만
        }
    }
    let rss_after = sysinfo::current_rss_bytes();
    let peak = sysinfo::peak_rss_bytes();
    println!("\n| memory | MB |\n|---|---:|");
    println!(
        "| working set after queries | {:.1} |",
        sysinfo::mb(rss_after)
    );
    println!("| peak working set (process) | {:.1} |", sysinfo::mb(peak));
    println!(
        "| engine-reported | {:.1} |",
        sysinfo::mb(idx.memory_bytes() as u64)
    );
    println!("(vectors in index: {})", idx.len().min(info.vectors));
    json.push(format!(
        "\"query\":{{\"mode\":\"{mode:?}\",\"open_secs\":{open_secs:.3},\"first_ms\":{first_ms:.1},\"ws_before_mb\":{:.1},\"ws_open_mb\":{:.1},\"ws_after_mb\":{:.1},\"peak_ws_mb\":{:.1},\"rows\":[{}]}}",
        sysinfo::mb(rss_before), sysinfo::mb(rss_open), sysinfo::mb(rss_after), sysinfo::mb(peak), rows.join(",")
    ));
}

/// 다중 프레임: 자산 단위 Recall@10 과 "자산 50개를 채우는 k".
fn multiframe(
    a: &args::Args,
    idx: &dyn AnnIndex,
    l: &Layout,
    queries: &[Vec<f32>],
    truth: &[Vec<u32>],
    info: &Info,
    json: &mut Vec<String>,
) {
    if !a.flag("multiframe") {
        return;
    }
    let ef: usize = a.num("mf-ef", 128);
    let conn = common::open_db(&l.db());
    let mut asset_of = vec![0u32; info.vectors + 1];
    {
        let mut st = conn
            .prepare("SELECT embedding_id, asset_id FROM poc_embeddings ORDER BY embedding_id")
            .unwrap();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            asset_of[r.get::<_, i64>(0).unwrap() as usize] = r.get::<_, i64>(1).unwrap() as u32;
        }
    }
    let lookup = |id: u64| {
        asset_of
            .get(id as usize)
            .map(|&x| x as u64)
            .filter(|&x| x != 0)
    };
    let p = SearchParams { ef_search: ef };
    // 정답 자산 top-10: 정답 프레임(상위 truth_k)을 자산으로 묶은 순서
    let truth_assets = |t: &[u32]| -> Vec<u64> {
        let hits: Vec<mam_vector::Hit> = t
            .iter()
            .enumerate()
            .map(|(i, &id)| mam_vector::Hit {
                id: id as u64,
                score: -(i as f32),
            })
            .collect();
        mam_vector::group_by_asset(&hits, &lookup)
            .iter()
            .map(|a| a.asset_id)
            .collect()
    };
    let (mut recall, mut ks, mut rounds_sum) = (0.0, Vec::new(), 0usize);
    let mut lat = Vec::new();
    for (q, t) in queries.iter().zip(truth) {
        let s = Instant::now();
        let r = search_assets(idx, q, 50, 50, &p, None, &lookup).unwrap();
        lat.push(s.elapsed());
        ks.push(r.k_used);
        rounds_sum += r.rounds;
        let want: Vec<u64> = truth_assets(t).into_iter().take(10).collect();
        let got: Vec<u64> = r.hits.iter().take(10).map(|h| h.asset_id).collect();
        recall += got.iter().filter(|x| want.contains(x)).count() as f64 / want.len().max(1) as f64;
    }
    ks.sort();
    let pk = |p: f64| ks[(((p / 100.0) * ks.len() as f64).ceil() as usize).clamp(1, ks.len()) - 1];
    let mut hist: HashMap<usize, usize> = HashMap::new();
    ks.iter().for_each(|k| *hist.entry(*k).or_default() += 1);
    let mut hv: Vec<_> = hist.into_iter().collect();
    hv.sort();
    let s = stats::summarize(&lat);
    println!(
        "\n| multi-frame (asset level, want 50 assets, initial k=50, ef={ef}) | value |\n|---|---:|"
    );
    println!(
        "| asset Recall@10 vs exact | {:.4} |",
        recall / queries.len() as f64
    );
    println!(
        "| k needed P50 / P95 / max | {} / {} / {} |",
        pk(50.0),
        pk(95.0),
        ks.last().unwrap()
    );
    println!(
        "| avg rounds | {:.2} |",
        rounds_sum as f64 / queries.len() as f64
    );
    println!(
        "| latency P50 / P95 / P99 ms | {:.2} / {:.2} / {:.2} |",
        s.p50_ms, s.p95_ms, s.p99_ms
    );
    println!("| k histogram | {hv:?} |");
    json.push(format!(
        "\"multiframe\":{{\"ef\":{ef},\"asset_recall10\":{:.4},\"k_p50\":{},\"k_p95\":{},\"k_max\":{},\"avg_rounds\":{:.2},\"lat\":{}}}",
        recall / queries.len() as f64, pk(50.0), pk(95.0), ks.last().unwrap(), rounds_sum as f64 / queries.len() as f64, s.json()
    ));
}
