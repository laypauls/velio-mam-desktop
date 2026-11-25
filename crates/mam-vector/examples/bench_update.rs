//! 증분 갱신·삭제·동시성 측정: 단건/배치 add, remove 후 검색 제외, 10% 삭제 후 크기·Recall, 재압축, add 중 검색 지연.
//!
//! 실행: `bench_update --dir <dataset> --engine flat --quant f16 [--ef 128]` (먼저 `bench_index --phase build` 로 인덱스를 만든다)
//! 인덱스 파일은 읽기만 하고, 삭제 후 크기 측정용 저장은 임시 폴더(`<dir>/tmp-update`)에 한다.

#[path = "common/mod.rs"]
mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::data::{self, Info, Layout};
use common::rng::Rng;
use common::{args, stats, sysinfo};
use mam_vector::{AnnIndex, OpenMode, SearchParams, add_parallel, open_index, save_index};

fn main() {
    let a = args::Args::parse();
    let l = Layout::new(
        a.path("dir")
            .unwrap_or_else(|| args::fail("--dir is required")),
    );
    let info = Info::read(&l);
    let spec = common::spec_from_args(&a, info.dim);
    let ef: usize = a.num("ef", 128);
    let p = SearchParams { ef_search: ef };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let name = common::index_name(&spec);
    println!("\n== bench_update {name} ef={ef} ==");
    let cond = common::print_conditions();
    println!("Dataset: vectors={} dim={}", info.vectors, info.dim);

    let queries = data::read_queries(&l, &info);
    let truth = data::read_truth(&l, &info);
    let t = Instant::now();
    let idx = open_index(&l.index(&name), &spec, OpenMode::Load)
        .unwrap_or_else(|e| args::fail(&format!("open: {e}")));
    let load_secs = t.elapsed().as_secs_f64();
    let n0 = idx.len();
    println!(
        "loaded {n0} vectors in {load_secs:.2}s, working set {:.1} MB",
        sysinfo::mb(sysinfo::current_rss_bytes())
    );

    // 기준 Recall@10 (삭제 전)
    let base_recall = mean_recall(&*idx, &queries, &truth, &p, &|_| true);

    // 1) 단건 add 200 + 배치 add 1000 (색인에 없는 질의 벡터를 새 id 로)
    let base_id = info.vectors as u64 + 1_000_000;
    let mut single = Vec::new();
    for (i, q) in queries.iter().enumerate().take(200) {
        let s = Instant::now();
        idx.add(base_id + i as u64, q).unwrap();
        single.push(s.elapsed());
    }
    let ss = stats::summarize(&single);
    let batch_ids: Vec<u64> = (200..1000u64).map(|i| base_id + i).collect();
    let batch_rows: Vec<f32> = queries[200..1000].iter().flatten().copied().collect();
    let t = Instant::now();
    add_parallel(&*idx, &batch_ids, &batch_rows, threads).unwrap();
    let batch_secs = t.elapsed().as_secs_f64();
    // 새로 넣은 벡터가 바로 검색된다
    let found_new = (0..100usize)
        .filter(|&i| {
            idx.search(&queries[i * 10], 1, &p, None).unwrap()[0].id == base_id + (i * 10) as u64
        })
        .count();
    println!("\n| add | value |\n|---|---:|");
    println!(
        "| single add P50 / P95 / P99 ms (n=200) | {:.2} / {:.2} / {:.2} |",
        ss.p50_ms, ss.p95_ms, ss.p99_ms
    );
    println!(
        "| batch add 800 vectors ({threads} threads) | {batch_secs:.2} s ({:.0}/s) |",
        800.0 / batch_secs
    );
    println!("| new vectors found as top-1 (of 100 probes) | {found_new} |");

    // 2) 방금 넣은 1000개 remove -> 검색에서 제외
    let t = Instant::now();
    let removed_new = (0..1000u64)
        .filter(|i| idx.remove(base_id + i).unwrap())
        .count();
    let remove_new_secs = t.elapsed().as_secs_f64();
    let leaked = (0..100usize)
        .filter(|&i| {
            idx.search(&queries[i], 10, &p, None)
                .unwrap()
                .iter()
                .any(|h| h.id >= base_id)
        })
        .count();
    println!(
        "| remove 1000 just-added | {remove_new_secs:.3} s ({removed_new} removed), probes still returning removed: {leaked} |"
    );

    // 3) 원본 10% 삭제
    let mut rng = Rng::new(7);
    let mut gone = vec![false; info.vectors + 1];
    let mut victims = Vec::new();
    while victims.len() < info.vectors / 10 {
        let id = rng.below(info.vectors) + 1;
        if !gone[id] {
            gone[id] = true;
            victims.push(id as u64);
        }
    }
    let t = Instant::now();
    victims.iter().for_each(|&id| {
        idx.remove(id).unwrap();
    });
    let remove_10_secs = t.elapsed().as_secs_f64();
    let alive = |id: u64| !gone[id as usize];
    // 정답에서 삭제된 것을 빼고 남은 상위 10 과 비교
    let after_recall = mean_recall(&*idx, &queries, &truth, &p, &alive);
    let leaked_10 = queries
        .iter()
        .take(200)
        .filter(|q| {
            idx.search(q, 10, &p, None)
                .unwrap()
                .iter()
                .any(|h| !alive(h.id))
        })
        .count();
    println!(
        "| remove 10% ({} vectors) | {remove_10_secs:.2} s ({:.0}/s) |",
        victims.len(),
        victims.len() as f64 / remove_10_secs
    );
    println!(
        "| Recall@10 before / after 10% delete (vs surviving exact top-10) | {base_recall:.4} / {after_recall:.4} |"
    );
    println!("| queries (of 200) returning a deleted vector | {leaked_10} |");
    println!("| len after delete | {} |", idx.len());

    // 4) 삭제 후 크기: 저장 -> (USearch 는) compact -> 다시 저장
    let tmp = l.dir.join("tmp-update");
    let _ = std::fs::remove_dir_all(&tmp);
    let t = Instant::now();
    save_index(&tmp, &*idx, &spec).unwrap();
    let save_secs = t.elapsed().as_secs_f64();
    let size_after_delete = data::dir_size(&tmp);
    let orig_size = data::dir_size(&l.index(&name));
    println!(
        "| index size original / after 10% delete (saved) MB | {:.1} / {:.1} (save {save_secs:.1}s) |",
        sysinfo::mb(orig_size),
        sysinfo::mb(size_after_delete)
    );
    let _ = std::fs::remove_dir_all(&tmp);
    run_concurrent(
        &*idx,
        &queries,
        &p,
        &info,
        &cond,
        &name,
        &[
            format!("\"load_secs\":{load_secs:.2}"),
            format!("\"single_add\":{}", ss.json()),
            format!("\"batch800_secs\":{batch_secs:.2}"),
            format!("\"remove10_secs\":{remove_10_secs:.2}"),
            format!("\"recall_before\":{base_recall:.4},\"recall_after_delete\":{after_recall:.4}"),
            format!(
                "\"size_orig_mb\":{:.1},\"size_after_delete_mb\":{:.1}",
                sysinfo::mb(orig_size),
                sysinfo::mb(size_after_delete)
            ),
        ],
    );
}

/// 정답(상위 truth_k) 중 `alive` 인 처음 10개와의 평균 Recall@10.
fn mean_recall(
    idx: &dyn AnnIndex,
    queries: &[Vec<f32>],
    truth: &[Vec<u32>],
    p: &SearchParams,
    alive: &dyn Fn(u64) -> bool,
) -> f64 {
    let mut sum = 0.0;
    for (q, t) in queries.iter().zip(truth) {
        let want: Vec<u32> = t
            .iter()
            .copied()
            .filter(|&id| alive(id as u64))
            .take(10)
            .collect();
        let got = idx.search(q, 10, p, None).unwrap();
        sum += got.iter().filter(|h| want.contains(&(h.id as u32))).count() as f64
            / want.len().max(1) as f64;
    }
    sum / queries.len() as f64
}

/// add 가 한창일 때 검색 지연 (쓰기 스레드 1개 + 검색 1개).
fn run_concurrent(
    idx: &dyn AnnIndex,
    queries: &[Vec<f32>],
    p: &SearchParams,
    info: &Info,
    cond: &sysinfo::Conditions,
    name: &str,
    json_parts: &[String],
) {
    let baseline: Vec<Duration> = queries
        .iter()
        .take(300)
        .map(|q| time(|| idx.search(q, 10, p, None).unwrap()))
        .collect();
    let base = stats::summarize(&baseline);
    let stop = AtomicBool::new(false);
    let base_id = info.vectors as u64 + 2_000_000;
    let mut adds = 0usize;
    let mut during: Vec<Duration> = Vec::new();
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            let mut n = 0usize;
            while !stop.load(Ordering::Relaxed) {
                idx.add(base_id + n as u64, &queries[n % queries.len()])
                    .unwrap();
                n += 1;
            }
            n
        });
        std::thread::sleep(Duration::from_millis(200));
        for q in queries.iter().cycle().take(600) {
            during.push(time(|| idx.search(q, 10, p, None).unwrap()));
        }
        stop.store(true, Ordering::Relaxed);
        adds = writer.join().unwrap();
    });
    let d = stats::summarize(&during);
    println!(
        "\n| search latency (k=10) | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|"
    );
    println!(
        "| idle | {:.2} | {:.2} | {:.2} | {:.2} |",
        base.p50_ms, base.p95_ms, base.p99_ms, base.max_ms
    );
    println!(
        "| while 1 thread adds continuously ({adds} adds) | {:.2} | {:.2} | {:.2} | {:.2} |",
        d.p50_ms, d.p95_ms, d.p99_ms, d.max_ms
    );
    println!(
        "peak working set: {:.1} MB",
        sysinfo::mb(sysinfo::peak_rss_bytes())
    );
    println!(
        "JSON: {{\"tool\":\"bench_update\",\"name\":{},\"vectors\":{},{},\"search_idle\":{},\"search_during_add\":{},\"adds_during\":{adds},\"conditions\":{}}}",
        common::json_str(name),
        info.vectors,
        json_parts.join(","),
        base.json(),
        d.json(),
        common::conditions_json(cond)
    );
}

fn time<T>(f: impl FnOnce() -> T) -> Duration {
    let s = Instant::now();
    let _ = f();
    s.elapsed()
}
