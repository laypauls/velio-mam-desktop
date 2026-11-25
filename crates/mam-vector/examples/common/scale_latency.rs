//! P4 규모 측정 — 질의 지연 (새 프로세스에서 view 로 열어 측정): 열기·첫 질의·asset_map·허용 집합·스레드별 질의·결합·상세 조회.

use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::{Duration, Instant};

use mam_vector::fusion::{FilterStrategy, choose_filter_strategy};
use mam_vector::{
    AnnIndex, AssetHit, EmbedStore, FlatIndex, Generations, ModelKey, OpenMode, RrfParams,
    SearchParams, SimilarQuery, active_candidates, find_similar, fuse_rrf, plan_candidates,
    vector_candidates,
};
use rusqlite::Connection;

use crate::common::rng::Rng;
use crate::common::stats::{self, Summary};
use crate::common::sysinfo::{current_rss_bytes, mb, peak_rss_bytes};
use crate::common::{self};
use crate::scale_build::{Paths, semantic_key, visual_key};
use crate::scale_data::{
    Mixture, Scale, allowed_assets, allowed_eids, item_queries, video_queries,
};

pub struct Opts {
    /// 전체 스레드 구성의 질의 수 / 나머지 스레드 구성의 질의 수
    pub queries: usize,
    pub sweep_queries: usize,
    /// 유사 미디어 질의 수 (전체 스레드 / 나머지)
    pub similar: usize,
    pub sweep_similar: usize,
    /// asset_map 반복 수, per-query 로드 e2e 반복 수
    pub map_runs: usize,
    pub e2e_runs: usize,
    pub warmup: usize,
    /// 측정할 스레드 수 목록 (비면 전체/4/2/1)
    pub threads: Vec<usize>,
}

type Map = HashMap<u64, u64>;

/// 현재 CPU 사용률(%) 3초 평균 — 다른 빌드 등 동시 부하 기록용.
pub fn load_percent() -> String {
    let script = "(1..3 | ForEach-Object { (Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter \"Name='_Total'\").PercentProcessorTime; Start-Sleep -Seconds 1 }) -join ','";
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

fn measure<Q>(qs: &[Q], warm: usize, mut f: impl FnMut(&Q)) -> Summary {
    qs.iter().take(warm).for_each(&mut f);
    let samples: Vec<Duration> = qs
        .iter()
        .skip(warm)
        .map(|q| {
            let t = Instant::now();
            f(q);
            t.elapsed()
        })
        .collect();
    stats::summarize(&samples)
}

fn row(label: &str, threads: usize, s: &Summary) {
    println!(
        "| {label} | {threads} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
        s.n, s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
}

fn header() {
    println!(
        "\n| 질의 | 스레드 | n | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|---:|"
    );
}

fn open(g: &Generations, k: &ModelKey) -> FlatIndex {
    g.open_active(k, OpenMode::View)
        .unwrap()
        .expect("no active generation")
}

/// 반복 측정 (질의 입력 없음).
fn repeat(runs: usize, mut f: impl FnMut()) -> Vec<Duration> {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect()
}

fn map_bytes(m: &Map) -> usize {
    // hashbrown: 버킷 수 = (용량 x 8/7) 올림 2의 거듭제곱, 버킷당 16B + 제어 1B
    (m.capacity() * 8 / 7).next_power_of_two() * 17
}

fn load_map(conn: &Connection, store: &EmbedStore, k: &ModelKey) -> Map {
    store.asset_map(conn, k).unwrap()
}

fn map_section(conn: &Connection, store: &EmbedStore, sc: &Scale, o: &Opts) -> (Map, Map) {
    println!("\n### (a) asset_map 로드 — 질의마다 SQLite 활성 행 전체를 읽는 비용");
    println!(
        "| 종류 | 행 수 | 첫 로드(새 프로세스) ms | 이후 P50 ms | P95 ms | max ms | 맵 메모리(추정 MB) | RSS 증가(첫 로드, MB) |\n|---|---:|---:|---:|---:|---:|---:|---:|"
    );
    let mut maps = Vec::new();
    for k in [visual_key(), semantic_key()] {
        let rss0 = current_rss_bytes();
        let t = Instant::now();
        let first_map = load_map(conn, store, &k);
        let first = t.elapsed();
        let rss1 = current_rss_bytes();
        let n = first_map.len();
        let bytes = map_bytes(&first_map);
        let mut last = first_map;
        let samples = repeat(o.map_runs, || last = load_map(conn, store, &k));
        let s = stats::summarize(&samples);
        println!(
            "| {} | {n} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
            k.kind.as_str(),
            first.as_secs_f64() * 1e3,
            s.p50_ms,
            s.p95_ms,
            s.max_ms,
            mb(bytes as u64),
            mb(rss1.saturating_sub(rss0))
        );
        maps.push(last);
    }
    let sem = maps.pop().unwrap();
    let vis = maps.pop().unwrap();
    // 적재 배치(산술)와 SQLite 맵 일치 검증
    assert_eq!(vis.len() as u64, sc.visual(), "visual map size");
    assert!(
        vis.iter().all(|(&e, &a)| a == sc.visual_asset(e)),
        "visual map content"
    );
    assert_eq!(sem.len() as u64, sc.assets(), "semantic map size");
    assert!(
        sem.iter().all(|(&e, &a)| a == sc.semantic_asset(e)),
        "semantic map content"
    );
    println!("(검증: 맵 크기·내용이 적재 배치와 전부 일치)");
    (vis, sem)
}

fn allowed_section(vis_map: &Map, sc: &Scale, o: &Opts) {
    println!("\n### (b) 허용 집합 구성 (자산 단위 허용 목록 -> 임베딩 id -> HashSet)");
    println!(
        "SQL 로 허용 자산 id 를 구하는 시간은 제외(키워드 측 필터 질의 몫). 아래는 그 결과를 받은 뒤의 순수 메모리 작업.\n"
    );
    println!(
        "| 허용 | 허용 자산 | 허용 벡터 | 전략 | 자산 HashSet ms (P50/P95) | 맵 스캔으로 임베딩 id 추출 ms (P50/P95) | 임베딩 id HashSet ms (P50/P95) | 합 P50 ms |\n|---|---:|---:|---|---|---|---|---:|"
    );
    for pct in [1u64, 10, 33] {
        let assets = allowed_assets(sc, pct);
        let mut aset = HashSet::new();
        let s0 = stats::summarize(&repeat(o.map_runs, || {
            aset = assets.iter().copied().collect();
        }));
        let mut eids = Vec::new();
        let s1 = stats::summarize(&repeat(o.map_runs, || {
            eids = vis_map
                .iter()
                .filter(|(_, a)| aset.contains(a))
                .map(|(&e, _)| e)
                .collect();
        }));
        let mut eset: HashSet<u64> = HashSet::new();
        let s2 = stats::summarize(&repeat(o.map_runs, || {
            eset = eids.iter().copied().collect();
        }));
        assert_eq!(eset.len(), allowed_eids(sc, pct).len());
        let strat = match choose_filter_strategy(eids.len(), vis_map.len()) {
            FilterStrategy::Subset => "Subset",
            FilterStrategy::FullScanPredicate => "FullScan",
        };
        println!(
            "| {pct}% | {} | {} | {strat} | {:.1} / {:.1} | {:.1} / {:.1} | {:.1} / {:.1} | {:.1} |",
            assets.len(),
            eids.len(),
            s0.p50_ms,
            s0.p95_ms,
            s1.p50_ms,
            s1.p95_ms,
            s2.p50_ms,
            s2.p95_ms,
            s0.p50_ms + s1.p50_ms + s2.p50_ms
        );
    }
}

fn detail_fetch(conn: &Connection, sc: &Scale, o: &Opts) {
    let sql = format!(
        "SELECT asset_id, media_type, status, file_size, modified_at FROM media_assets WHERE asset_id IN ({})",
        vec!["?"; 50].join(",")
    );
    let mut rng = Rng::new(7);
    let qs: Vec<Vec<i64>> = (0..o.queries + o.warmup)
        .map(|_| {
            (0..50)
                .map(|_| 1 + rng.below(sc.assets() as usize) as i64)
                .collect()
        })
        .collect();
    let mut st = conn.prepare_cached(&sql).unwrap();
    let s = measure(&qs, o.warmup, |ids| {
        let n = st
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            .count();
        std::hint::black_box(n);
    });
    row(
        "SQLite 상세 50건 (`media_assets` 한 테이블, IN 50개)",
        1,
        &s,
    );
}

struct Ctx<'a> {
    vis: &'a FlatIndex,
    sem: &'a FlatIndex,
    vis_map: &'a Map,
    sem_map: &'a Map,
}

fn sweep(c: &Ctx, sc: &Scale, threads: usize, n: usize, n_sim: usize, o: &Opts) {
    c.vis.set_threads(threads);
    c.sem.set_threads(threads);
    let plan = plan_candidates(50);
    let w = o.warmup;
    let vq = item_queries(&Mixture::visual(), n + w);
    let sq = item_queries(&Mixture::semantic(), n + w);
    let vis_of = |e: u64| c.vis_map.get(&e).copied();
    let sem_of = |e: u64| c.sem_map.get(&e).copied();
    let total = c.vis.len();

    let s = measure(&vq, w, |q| {
        std::hint::black_box(vector_candidates(c.vis, q, &plan, None, total, &vis_of).unwrap());
    });
    row("시각 512-d 무필터 (후보 400, 자산 단위)", threads, &s);
    for pct in [1u64, 10, 33] {
        let ids = allowed_eids(sc, pct);
        let strat = choose_filter_strategy(ids.len(), total);
        let s = measure(&vq, w, |q| {
            std::hint::black_box(
                vector_candidates(c.vis, q, &plan, Some(&ids), total, &vis_of).unwrap(),
            );
        });
        row(
            &format!("시각 허용 {pct}% ({strat:?}, 선적용)"),
            threads,
            &s,
        );
    }
    let vqs = video_queries(&Mixture::visual(), sc, n_sim + w);
    let s = measure(&vqs, w, |(a, frames)| {
        let q = SimilarQuery {
            frames,
            exclude: std::slice::from_ref(a),
            limit: 50,
            cursor: None,
            min_score: 0.0,
        };
        std::hint::black_box(find_similar(c.vis, &q, &vis_of).unwrap());
    });
    row("유사 미디어 (질의 프레임 5개, 50건)", threads, &s);
    let s = measure(&sq, w, |q| {
        std::hint::black_box(
            vector_candidates(c.sem, q, &plan, None, c.sem.len(), &sem_of).unwrap(),
        );
    });
    row("의미 384-d 무필터 (후보 400)", threads, &s);
}

/// 전체 스레드에서만: 전략 강제 비교, per-query 맵 로드 e2e, 결합, 상세 조회.
fn extras(conn: &Connection, store: &EmbedStore, c: &Ctx, sc: &Scale, threads: usize, o: &Opts) {
    let w = o.warmup;
    let n = o.queries;
    let vq = item_queries(&Mixture::visual(), n + w);
    let params = SearchParams::default();
    println!("\n#### 필터 전략 강제 비교 (임베딩 단위 k=400, 자산 묶기 제외, {threads} 스레드)");
    header();
    for pct in [1u64, 10, 33] {
        let ids = allowed_eids(sc, pct);
        let s = measure(&vq, w, |q| {
            std::hint::black_box(c.vis.search_subset(q, 400, &ids).unwrap());
        });
        row(&format!("허용 {pct}% 강제 Subset"), threads, &s);
        let s = measure(&vq, w, |q| {
            let set: HashSet<u64> = ids.iter().copied().collect();
            let pred = |id: u64| set.contains(&id);
            std::hint::black_box(c.vis.search(q, 400, &params, Some(&pred)).unwrap());
        });
        row(
            &format!("허용 {pct}% 강제 FullScan(집합 구성 포함)"),
            threads,
            &s,
        );
    }

    println!(
        "\n#### 현재 앱 경로: 질의마다 `active_candidates` (asset_map 전체 로드 포함, {threads} 스레드)"
    );
    header();
    let plan = plan_candidates(50);
    let vk = visual_key();
    let qs = &vq[..o.e2e_runs + 2];
    let s = measure(qs, 2, |q| {
        std::hint::black_box(active_candidates(conn, store, &vk, c.vis, q, &plan, None).unwrap());
    });
    row("active_candidates 무필터 (맵 로드 포함)", threads, &s);
    let ids10 = allowed_eids(sc, 10);
    let s = measure(qs, 2, |q| {
        std::hint::black_box(
            active_candidates(conn, store, &vk, c.vis, q, &plan, Some(&ids10)).unwrap(),
        );
    });
    row("active_candidates 허용 10% (맵 로드 포함)", threads, &s);

    println!(
        "\n#### 유사 미디어 진단: 프레임 1개 단일 스캔 비용 (k=204 = find_similar 첫 라운드, {threads} 스레드)"
    );
    header();
    let s = measure(&vq, w, |q| {
        std::hint::black_box(c.vis.search(q, 204, &params, None).unwrap());
    });
    row("스캔 1회 필터 없음", threads, &s);
    let s = measure(&vq, w, |q| {
        let keep = |id: u64| c.vis_map.get(&id).is_some_and(|a| *a != 0);
        std::hint::black_box(c.vis.search(q, 204, &params, Some(&keep)).unwrap());
    });
    row(
        "스캔 1회 필터=asset_map 조회 (find_similar 의 keep 과 같은 형태)",
        threads,
        &s,
    );

    println!("\n#### 결합·상세 ({threads} 스레드 / 1 스레드)");
    header();
    let vis_of = |e: u64| c.vis_map.get(&e).copied();
    let hits: Vec<AssetHit> =
        vector_candidates(c.vis, &vq[0], &plan, None, c.vis.len(), &vis_of).unwrap();
    let mut rng = Rng::new(11);
    let kw: Vec<u64> = (0..400)
        .map(|i| {
            if i % 2 == 0 {
                hits[(i / 2) % hits.len()].asset_id
            } else {
                1 + rng.below(sc.assets() as usize) as u64
            }
        })
        .collect();
    let idx: Vec<usize> = (0..1000 + w).collect();
    let p = RrfParams::default();
    let s = measure(&idx, w, |_| {
        std::hint::black_box(fuse_rrf(&kw, &hits, &p));
    });
    row(
        &format!("fuse_rrf 키워드 400 + 벡터 {} (중복 약 절반)", hits.len()),
        1,
        &s,
    );
    detail_fetch(conn, sc, o);
}

pub fn run(p: &Paths, sc: Scale, o: &Opts) {
    let all = std::thread::available_parallelism().map_or(4, |n| n.get());
    println!("CPU 부하(시작 전 3초, %): {}", load_percent());
    let conn = common::open_db(&p.db());
    let store = EmbedStore::new(p.vectors()).unwrap();
    let (vk, sk) = (visual_key(), semantic_key());
    let (vg, sg) = (
        Generations::new(&store, &vk).unwrap(),
        Generations::new(&store, &sk).unwrap(),
    );

    println!("\n### 열기 (view = 메모리 매핑) · 첫 질의");
    println!("RSS(시작): {:.0} MB", mb(current_rss_bytes()));
    let t = Instant::now();
    let vis = open(&vg, &vk);
    let open_first = t.elapsed();
    let t = Instant::now();
    let sem = open(&sg, &sk);
    let open_sem = t.elapsed();
    println!(
        "첫 열기(새 프로세스): 시각 {} 벡터 {:.1} ms, 의미 {} 벡터 {:.1} ms, RSS {:.0} MB",
        vis.len(),
        open_first.as_secs_f64() * 1e3,
        sem.len(),
        open_sem.as_secs_f64() * 1e3,
        mb(current_rss_bytes())
    );
    let reopen = stats::summarize(&repeat(10, || drop(open(&vg, &vk))));
    println!(
        "재열기(웜) 시각: P50 {:.1} / P95 {:.1} ms",
        reopen.p50_ms, reopen.p95_ms
    );
    let q0 = &item_queries(&Mixture::visual(), 1)[0];
    let t = Instant::now();
    let h = vis.search(q0, 400, &SearchParams::default(), None).unwrap();
    println!(
        "첫 질의(열기 직후, 시각 무필터 k=400, {all} 스레드): {:.1} ms (hits {}), RSS {:.0} MB",
        t.elapsed().as_secs_f64() * 1e3,
        h.len(),
        mb(current_rss_bytes())
    );
    let sq0 = &item_queries(&Mixture::semantic(), 1)[0];
    let t = Instant::now();
    sem.search(sq0, 400, &SearchParams::default(), None)
        .unwrap();
    println!(
        "첫 질의(의미): {:.1} ms, RSS {:.0} MB",
        t.elapsed().as_secs_f64() * 1e3,
        mb(current_rss_bytes())
    );

    let (vis_map, sem_map) = map_section(&conn, &store, &sc, o);
    allowed_section(&vis_map, &sc, o);
    let c = Ctx {
        vis: &vis,
        sem: &sem,
        vis_map: &vis_map,
        sem_map: &sem_map,
    };

    println!("\n### 스레드 수별 질의 지연 (캐시된 asset_map 사용 = 세대 캐시 가정)");
    header();
    let mut counts = if o.threads.is_empty() {
        vec![all, 4, 2, 1]
    } else {
        o.threads.clone()
    };
    counts.dedup();
    for (i, &t) in counts.iter().enumerate() {
        println!("(CPU 부하 %, 이 구성 시작 전: {})", load_percent());
        let (n, ns) = if i == 0 {
            (o.queries, o.similar)
        } else {
            (o.sweep_queries, o.sweep_similar)
        };
        sweep(&c, &sc, t, n, ns, o);
    }
    c.vis.set_threads(all);
    c.sem.set_threads(all);
    extras(&conn, &store, &c, &sc, all, o);
    println!(
        "\n최대 RSS(latency 프로세스): {:.0} MB, 종료 시 RSS {:.0} MB",
        mb(peak_rss_bytes()),
        mb(current_rss_bytes())
    );
    println!("CPU 부하(종료 후 3초, %): {}", load_percent());
}
