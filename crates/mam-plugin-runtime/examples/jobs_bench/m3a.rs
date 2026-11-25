//! M3(a) — DB 한정: outbox 이벤트 1M 을 변경 피드(`PluginJobs::feed_tick`, FEED_BATCH 2,000)가 따라잡는 비용과, 동시 스캔 쓰기 부하
//! 모사 중 쓰기 점유율·High 쓰기 지연. 스캔 모사는 실제 수집 쓰기의 대용품이다(`scan/work.rs` 의 500건 Low 트랜잭션 모양:
//! 자산 + 위치 + outbox upsert + meta/thumb/hash `jobs`) — 실제 수집 코드가 아니다(그것은 M3(b)).
//! 피드 backpressure(PENDING_CAP)는 끈다 — 실행기가 없는 DB 한정 측정이라 대기 작업이 쌓이기만 한다.
//! 단계 4 는 단계마다 새 플러그인(B, C)을 설치해 커서를 0 으로 되감는다(작업 행 삽입 경로를 같게 — 이전 플러그인은 꺼서 피드에서 뺀다).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use mam_db::plugin_tasks::{feed_cursor, max_event_id, set_feed_cursor};
use mam_db::{DbError, DbPool, Priority};
use mam_plugin_runtime::jobs::JobsTuning;

use crate::env::{Env, PLUGIN_A, PLUGIN_B, PLUGIN_C};
use crate::synth;
use crate::util::{Lat, Load, db_files_bytes, mib, say};

const SCAN_BATCH: usize = 500;
/// 스캔 모사 배치 번호 — 단계가 바뀌어도 경로(`sim/<seq>/<i>.jpg`)가 겹치지 않게 프로세스 전체에서 이어간다
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 스캔 모사 한 배치: 500 파일 = 자산·위치·outbox·`jobs` 3종을 한 Low 트랜잭션에
fn scan_batch(pool: &DbPool, seq: u64) -> Result<(), String> {
    pool.write(Priority::Low, move |c| {
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        {
            let mut a = tx.prepare_cached("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES ('image', 1, 1, 1, 1)")?;
            let mut l = tx.prepare_cached("INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, 1, ?2, ?2, 1)")?;
            let mut o = tx.prepare_cached("INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at) VALUES (?1, 1, 'upsert', 1, 1)")?;
            let mut j = tx.prepare_cached("INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at) VALUES (?1, ?2, 50, 1, 1)")?;
            for i in 0..SCAN_BATCH {
                a.execute([])?;
                let id = tx.last_insert_rowid();
                l.execute(rusqlite::params![id, format!("sim/{seq}/{i}.jpg")])?;
                o.execute([id])?;
                for kind in ["meta", "thumb", "hash"] {
                    j.execute(rusqlite::params![kind, id])?;
                }
            }
        }
        tx.commit()?;
        Ok::<(), DbError>(())
    })
    .map_err(|e| e.to_string())
}

struct Scanner {
    stop: Arc<AtomicBool>,
    files: Arc<AtomicU64>,
    handle: std::thread::JoinHandle<Lat>,
}

/// `rate` = 목표 파일/초 (없으면 쉬지 않고 — 쓰기 스레드를 포화시키는 최악 경우)
fn start_scanner(pool: Arc<DbPool>, rate: Option<u64>) -> Scanner {
    let (stop, files) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
    let (s, f) = (Arc::clone(&stop), Arc::clone(&files));
    let handle = std::thread::spawn(move || {
        let mut lat = Lat::default();
        let interval = rate.map(|r| Duration::from_secs_f64(SCAN_BATCH as f64 / r as f64));
        while !s.load(Ordering::Relaxed) {
            let t = Instant::now();
            scan_batch(&pool, SEQ.fetch_add(1, Ordering::Relaxed)).expect("scan batch");
            lat.push(t.elapsed());
            f.fetch_add(SCAN_BATCH as u64, Ordering::Relaxed);
            if let Some(iv) = interval {
                std::thread::sleep(iv.saturating_sub(t.elapsed()));
            }
        }
        lat
    });
    Scanner { stop, files, handle }
}

impl Scanner {
    fn finish(self) -> (Lat, u64) {
        self.stop.store(true, Ordering::Relaxed);
        let lat = self.handle.join().expect("scanner");
        (lat, self.files.load(Ordering::Relaxed))
    }
}

fn scan_report(label: &str, lat: &Lat, files: u64, wall: Duration) {
    say(lat.line(&format!("  [{label}] sim-scan Low tx (500 files)")));
    say(format!("  [{label}] sim-scan throughput {:.0} files/s ({files} files, {:.1}s); scan-tx writer occupancy {:.1}%", files as f64 / wall.as_secs_f64(), wall.as_secs_f64(), 100.0 * lat.sum_ms() / (wall.as_secs_f64() * 1000.0)));
}

/// 실제 `feed_tick`(+ 실제 스레드와 같은 50 ms 양보)을 `window` 동안(없으면 따라잡을 때까지) 돈다. (틱별 시간, 전체 시간)
fn run_feed(env: &Env, window: Option<Duration>) -> (Lat, Duration) {
    let (mut ticks, t0) = (Lat::default(), Instant::now());
    loop {
        let t = Instant::now();
        let more = env.jobs.feed_tick();
        ticks.push(t.elapsed());
        if window.map_or(!more, |w| t0.elapsed() >= w) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (ticks, t0.elapsed())
}

fn cursor(env: &Env, plugin: &'static str) -> i64 {
    env.pool.read(|c| Ok(feed_cursor(c, plugin)?)).expect("cursor").unwrap_or(-1)
}

fn rewind(env: &Env, plugin: &'static str) {
    env.pool
        .write(Priority::High, move |c| {
            let tx = c.transaction()?;
            set_feed_cursor(&tx, plugin, 0).map_err(|e| DbError::Io(std::io::Error::other(e.to_string())))?;
            tx.commit()?;
            Ok(())
        })
        .expect("rewind cursor");
}

fn feed_report(label: &str, env: &Env, plugin: &'static str, ticks: &Lat, wall: Duration, high: &Lat) {
    say(format!("  [{label}] feed cursor {} / max(event_id) {} after {:.1}s ({} ticks) => {:.0} events/s", cursor(env, plugin), env.pool.read(|c| Ok(max_event_id(c)?)).expect("max"), wall.as_secs_f64(), ticks.0.len(), cursor(env, plugin) as f64 / wall.as_secs_f64()));
    say(ticks.line(&format!("  [{label}] per feed_tick (read + 1 Low tx of up to 2,000 event ids)")));
    say(format!("  [{label}] feed writer occupancy = {:.1}% of wall (tick time / wall; ticks include a small read)", 100.0 * ticks.sum_ms() / (wall.as_secs_f64() * 1000.0)));
    say(high.line(&format!("  [{label}] High write")));
}

/// 단계 4 한 번: 새 플러그인을 설치·되감고, 스캔 모사(`rate`)와 피드를 `window` 동안 같이 돌린다
fn scan_plus_feed(env: &Env, label: &str, plugin: &'static str, prev: &'static str, rate: Option<u64>, window: Duration, seed: u64) {
    let _ = env.rt.set_enabled(prev, false, "en");
    env.install(plugin);
    rewind(env, plugin);
    let w = Load::spawn(Duration::from_millis(10), env.editor(1_000_000, seed));
    let (sc, t) = (start_scanner(Arc::clone(&env.pool), rate), Instant::now());
    let (ticks, feed_wall) = run_feed(env, Some(window));
    let wall = t.elapsed();
    let (lat, files) = sc.finish();
    let high = w.finish().0;
    scan_report(label, &lat, files, wall);
    feed_report(label, env, plugin, &ticks, feed_wall, &high);
    say(format!("  [{label}] combined writer occupancy (scan tx + feed ticks) = {:.1}% of wall", 100.0 * (ticks.sum_ms() + lat.sum_ms()) / (wall.as_secs_f64() * 1000.0)));
}

pub fn run(scratch: &Path, assets: u64, events: u64) {
    let base = scratch.join("m3a");
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("MAMData");
    let t = Instant::now();
    let db = synth::build(&root, assets, true, events);
    say(format!("M3a dataset: {assets} assets + {assets} locations + {events} outbox upsert events, built in {:.1}s; db {:.1} MiB", t.elapsed().as_secs_f64(), mib(db_files_bytes(&db))));
    let env = Env::open(&root, None, None);
    env.install(PLUGIN_A);
    rewind(&env, PLUGIN_A);
    say(format!("  feed cursor rewound to 0; max(event_id)={}; FEED_BATCH={} cap off", env.pool.read(|c| Ok(max_event_id(c)?)).expect("max"), mam_plugin_runtime::jobs::FEED_BATCH));
    env.jobs.set_tuning(JobsTuning { pending_cap: i64::MAX, ..JobsTuning::default() });
    let hi_pause = Duration::from_millis(10);

    // 1) High 쓰기 기준선
    let w = Load::spawn(hi_pause, env.editor(assets, 21));
    std::thread::sleep(Duration::from_secs(6));
    say(w.finish().0.line("  [1 idle] High write baseline (no scan, no feed)"));

    // 2) 스캔 모사만 (피드 없음) — 포화, 그리고 2,000 파일/초
    for (label, rate, seed) in [("2a scan only, saturating", None, 22), ("2b scan only, 2000 files/s", Some(2000), 25)] {
        let w = Load::spawn(hi_pause, env.editor(assets, seed));
        let (sc, t) = (start_scanner(Arc::clone(&env.pool), rate), Instant::now());
        std::thread::sleep(Duration::from_secs(20));
        let wall = t.elapsed();
        let (lat, files) = sc.finish();
        scan_report(label, &lat, files, wall);
        say(w.finish().0.line(&format!("  [{label}] High write")));
    }

    // 3) 피드만 (플러그인 A, 1M+ 이벤트 따라잡기)
    let w = Load::spawn(hi_pause, env.editor(assets, 23));
    let (ticks, wall) = run_feed(&env, None);
    let high = w.finish().0;
    feed_report("3 feed only, drain", &env, PLUGIN_A, &ticks, wall, &high);
    say(format!("  rows now: plugin_tasks={} jobs(plugin)={}", env.count("SELECT COUNT(*) FROM plugin_tasks"), env.count("SELECT COUNT(*) FROM jobs WHERE kind='plugin'")));

    // 4) 스캔 모사 + 피드 — 60 초 창 (포화 스캔은 피드가 영영 못 따라잡을 수 있어 고정 창으로 본다)
    scan_plus_feed(&env, "4a scan saturating + feed", PLUGIN_B, PLUGIN_A, None, Duration::from_secs(60), 24);
    scan_plus_feed(&env, "4b scan 2000 files/s + feed", PLUGIN_C, PLUGIN_B, Some(2000), Duration::from_secs(60), 26);

    env.shutdown();
    drop(env);
    let _ = std::fs::remove_dir_all(&base);
    say("M3a cleanup: dataset directory deleted");
}
