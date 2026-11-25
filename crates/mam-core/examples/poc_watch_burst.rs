//! 변경 대기열 이벤트 폭주 측정 (멀티 스캔 0단계 c) — 실제 `ChangeSink` -> 기록기 -> `watch_pending` 경로에
//! 서로 다른 경로 N 개를 한꺼번에 보고하고, 그동안 사용자 쓰기(별점, High)와 목록 조회(하이드레이션)의 지연을 함께 잰다.
//!
//! ```text
//! cargo run --release -p mam-core --example poc_watch_burst -- <lib-dir> <burst-dir> <data-dir> <paths> <rate/s|0> <processor on|off> <cap>
//! ```
//!
//! - `<lib-dir>`: 자산을 채울 합성 폴더(`bench_scan gen` 으로 만든 것), `<burst-dir>`: 보고 대상 관리 폴더(빈 폴더), `<data-dir>`: DB 위치(로컬 SSD)
//! - `rate` 0 = 제한 없이 한 번에 보고. `processor off` 는 처리기가 행을 건드리지 않게(디바운스 1시간) 기록기만 잰다
//! - 보고하는 경로는 실제로 없는 파일이다(대기열 쓰기만 잰다)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps, PendingConfig};
use mam_core::query::QueryService;
use mam_core::scan::scan_folder;
use mam_core::userdata::UserData;
use mam_core::watch::{
    ChangeDetector, ChangeSink, DetectError, DetectorOptions, DetectorRegistry, ProbeResult,
    WatchHandle, WatchTarget,
};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, Priority, SystemClock, migrate, open};

type Sinks = Arc<Mutex<HashMap<i64, ChangeSink>>>;

/// 시작할 때 받은 sink 를 폴더별로 보관한다 (실제 `WatchManager` 가 만든 sink — 기록기 경로가 같다)
struct Probe(Sinks);
struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Probe {
    fn id(&self) -> &'static str {
        "burst_probe"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        self.0.lock().unwrap().insert(t.folder_id, sink);
        Ok(Box::new(Handle))
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[(((v.len() - 1) as f64) * p).round() as usize]
}

fn row(label: &str, v: &mut [f64]) {
    println!(
        "| {label} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
        v.len(),
        pct(v, 0.5),
        pct(v, 0.95),
        pct(v, 0.99),
        pct(v, 1.0)
    );
}

/// 주기적으로 `f` 를 불러 (시작 시각, 걸린 ms) 를 모은다
fn probe_loop(
    stop: Arc<AtomicBool>,
    every: Duration,
    mut f: impl FnMut() + Send + 'static,
) -> std::thread::JoinHandle<Vec<(Instant, f64)>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            let t = Instant::now();
            f();
            out.push((t, t.elapsed().as_secs_f64() * 1000.0));
            std::thread::sleep(every);
        }
        out
    })
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (lib, burst, data) = (
        PathBuf::from(&a[0]),
        PathBuf::from(&a[1]),
        PathBuf::from(&a[2]),
    );
    let (paths, rate): (usize, usize) = (a[3].parse().expect("paths"), a[4].parse().expect("rate"));
    let processor = a[5] == "on";
    let cap: usize = a[6].parse().expect("cap");

    std::fs::create_dir_all(&data).expect("data dir");
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).expect("open");
    migrate(&mut conn, &data.join("backups"), &SystemClock).expect("migrate");
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 4).expect("pool"));
    let sinks: Sinks = Arc::default();
    let clock: SharedClock = Arc::new(SystemClock);
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&pool),
            clock: Arc::clone(&clock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![data.clone()],
            debounce_ms: if processor { 2_000 } else { 3_600_000 },
            pending: PendingConfig {
                max_rows_per_folder: cap,
                ..PendingConfig::default()
            },
            detectors: Arc::new(DetectorRegistry::new(
                vec![Arc::new(Probe(Arc::clone(&sinks)))],
                &["burst_probe"],
            )),
            ..IngestConfig::default()
        },
    );

    // 자산 채우기 (목록 조회용) — 스레드를 시작하기 전에 직접 스캔한다
    let lib_id = ingest
        .add_folder(lib.to_str().unwrap())
        .expect("add lib")
        .folder_id;
    let burst_id = ingest
        .add_folder(burst.to_str().unwrap())
        .expect("add burst")
        .folder_id;
    let t = Instant::now();
    scan_folder(ingest.env(), lib_id, &|| false, &AtomicBool::new(false)).expect("scan");
    let assets: i64 = pool
        .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))?))
        .expect("count");
    println!("seeded {assets} assets in {:?}", t.elapsed());
    pool.write(Priority::Low, |c| Ok(c.execute("DELETE FROM jobs", [])?))
        .expect("clear jobs");
    ingest.start().expect("start");
    pool.write(Priority::Low, |c| Ok(c.execute("DELETE FROM jobs", [])?))
        .expect("clear jobs");
    let sink = sinks
        .lock()
        .unwrap()
        .get(&burst_id)
        .cloned()
        .expect("sink for the burst folder");
    let ids: Vec<i64> = pool
        .read(|c| {
            let mut s =
                c.prepare("SELECT asset_id FROM media_assets ORDER BY asset_id LIMIT 2000")?;
            Ok(s.query_map([], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .expect("ids");

    // 프로브: 사용자 쓰기(별점 — 자산 변경 + outbox 한 트랜잭션, High) 와 목록 조회 (페이지 100건 하이드레이션)
    let ud = Arc::new(UserData::new(Arc::clone(&pool), Arc::clone(&clock)));
    let q = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
    let spawn_probes = |stop: &Arc<AtomicBool>| {
        let (ids_w, ids_r, ud, q) = (ids.clone(), ids.clone(), Arc::clone(&ud), Arc::clone(&q));
        let mut n = 0usize;
        let w = probe_loop(Arc::clone(stop), Duration::from_millis(10), move || {
            n += 1;
            ud.set_rating(&[ids_w[n % ids_w.len()]], (n % 5 + 1) as u8)
                .expect("rating");
        });
        let mut k = 0usize;
        let r = probe_loop(Arc::clone(stop), Duration::from_millis(10), move || {
            k += 100;
            let page: Vec<i64> = (0..100).map(|j| ids_r[(k + j) % ids_r.len()]).collect();
            q.hydrate(&page).expect("hydrate");
        });
        (w, r)
    };

    // 1) 대조: 폭주 없이 5초
    let stop = Arc::new(AtomicBool::new(false));
    let (w, r) = spawn_probes(&stop);
    std::thread::sleep(Duration::from_secs(5));
    stop.store(true, Ordering::Relaxed);
    let (mut idle_w, mut idle_r): (Vec<f64>, Vec<f64>) = (
        w.join().unwrap().iter().map(|x| x.1).collect(),
        r.join().unwrap().iter().map(|x| x.1).collect(),
    );

    // 2) 폭주: 기록 완료 관찰(2ms 간격 COUNT) + 프로브 + 보고
    let stop = Arc::new(AtomicBool::new(false));
    let (w, r) = spawn_probes(&stop);
    let mon_stop = Arc::new(AtomicBool::new(false));
    let mon = {
        let (pool, stop) = (Arc::clone(&pool), Arc::clone(&mon_stop));
        std::thread::spawn(move || {
            let mut s: Vec<(Instant, i64)> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let n: i64 = pool
                    .read(|c| {
                        Ok(c.query_row("SELECT COUNT(*) FROM watch_pending", [], |r| r.get(0))?)
                    })
                    .unwrap_or(-1);
                s.push((Instant::now(), n));
                std::thread::sleep(Duration::from_millis(2));
            }
            s
        })
    };
    let path_of = |i: usize| {
        burst
            .join(format!("d{:03}", i / 1000))
            .join(format!("f{i:06}.jpg"))
    };
    let started = Instant::now();
    let mut emitted_at = Vec::with_capacity(paths);
    for i in 0..paths {
        emitted_at.push(Instant::now());
        sink.path(path_of(i));
        if rate > 0 && i % 100 == 99 {
            let due = started + Duration::from_secs_f64((i + 1) as f64 / rate as f64);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    }
    let emit_done = started.elapsed();
    // 기록이 끝나길 기다린다 (상한 때문에 줄어드는 경우를 위해 개수가 멈추면 끝)
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut last = (-1i64, Instant::now());
    let rows = loop {
        let n: i64 = pool
            .read(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM watch_pending WHERE folder_id = ?1",
                    [burst_id],
                    |r| r.get(0),
                )?)
            })
            .expect("count");
        if n as usize >= paths
            || Instant::now() > deadline
            || (n == last.0 && last.1.elapsed() > Duration::from_secs(3) && !processor)
        {
            break n;
        }
        if n != last.0 {
            last = (n, Instant::now());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let total = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    let during = (started, started + total);
    let in_burst = |v: Vec<(Instant, f64)>| -> Vec<f64> {
        v.into_iter()
            .filter(|(t, _)| *t >= during.0 && *t <= during.1)
            .map(|x| x.1)
            .collect()
    };
    let (mut burst_w, mut burst_r) = (in_burst(w.join().unwrap()), in_burst(r.join().unwrap()));
    mon_stop.store(true, Ordering::Relaxed);
    let samples = mon.join().unwrap();

    println!(
        "\nburst: paths={paths} rate={rate}/s processor={processor} cap={cap} -> rows in watch_pending for the folder={rows}"
    );
    println!(
        "emit loop {:.0} ms, report -> all rows committed {:.0} ms total",
        emit_done.as_secs_f64() * 1000.0,
        total.as_secs_f64() * 1000.0
    );
    let jobs: i64 = pool
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap_or(-1);
    println!("pending scan jobs after the burst (cap overflow -> rescan): {jobs}");

    // 행별 기록 지연 — 같은 first_seen_at(= 한 번의 기록 묶음) 끼리 묶어, 묶음의 누적 행 수에 처음 닿은 관찰 시각을 커밋 시각으로 본다
    if !processor {
        let rows_v: Vec<(String, i64)> = pool
            .read(|c| {
                let mut s = c.prepare(
                    "SELECT abs_path, first_seen_at FROM watch_pending WHERE folder_id = ?1",
                )?;
                Ok(s.query_map([burst_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?)
            })
            .expect("rows");
        let mut by_batch: std::collections::BTreeMap<i64, Vec<usize>> = Default::default();
        for (p, t) in &rows_v {
            let i: usize = p
                .rsplit(['\\', '/'])
                .next()
                .and_then(|n| {
                    n.trim_start_matches('f')
                        .trim_end_matches(".jpg")
                        .parse()
                        .ok()
                })
                .unwrap_or(0);
            by_batch.entry(*t).or_default().push(i);
        }
        let (mut lat, mut cum) = (Vec::new(), 0i64);
        for (n, (_, members)) in by_batch.iter().enumerate() {
            cum += members.len() as i64;
            let commit = samples.iter().find(|s| s.1 >= cum).map(|s| s.0);
            println!(
                "batch {n}: {} rows (committed {:.0} ms after the first report)",
                members.len(),
                commit.map_or(f64::NAN, |c| c.duration_since(started).as_secs_f64()
                    * 1000.0)
            );
            if let Some(c) = commit {
                lat.extend(
                    members
                        .iter()
                        .map(|&i| c.duration_since(emitted_at[i]).as_secs_f64() * 1000.0),
                );
            }
        }
        println!(
            "\n| report -> committed row (ms) | n | P50 | P95 | P99 | max |\n|---|---|---|---|---|---|"
        );
        row("writer latency", &mut lat);
    }
    println!("\n| probe latency (ms) | n | P50 | P95 | P99 | max |\n|---|---|---|---|---|---|");
    row("user write, idle (5 s)", &mut idle_w);
    row("user write, during burst", &mut burst_w);
    row("list page (100 rows), idle (5 s)", &mut idle_r);
    row("list page (100 rows), during burst", &mut burst_r);

    ingest.shutdown();
    pool.shutdown();
    let _ = Path::new(&data);
}
