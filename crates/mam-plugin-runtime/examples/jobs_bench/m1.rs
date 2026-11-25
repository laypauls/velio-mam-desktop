//! M1 — 활성 자산 1M 에 provider 1개 run-all 뒤 `plugin_tasks`·`jobs` 크기(dbstat), run-all 소요, 배치 트랜잭션 P50/P95/P99,
//! run-all 동안 High 읽기·쓰기 지연. 1차 실행 = 실제 `PluginJobs::run_all`(플러그인 A), 2차 실행 = 같은 SQL·같은 Low 쓰기 경로의
//! 배치 루프(플러그인 B, 배치마다 시간 측정 — 실제 경로는 배치 시간을 밖으로 내지 않는다).

use std::path::Path;
use std::time::{Duration, Instant};

use mam_core::jobs::JobKind;
use mam_db::plugin_settings::{self, INSTALLED};
use mam_db::plugin_tasks::{RUN_ALL_BATCH, enqueue_jobs_tx, run_all_batch, with_immediate_tx};
use mam_db::Priority;

use crate::env::{Env, PLUGIN_A, PLUGIN_B, PROVIDER};
use crate::util::{Lat, Load, db_files_bytes, mib, report_pair, say};
use crate::synth;

const OBJECTS: [(&str, &str); 11] = [
    ("plugin_tasks", "table"),
    ("uq_plugin_tasks_key", "index"),
    ("idx_plugin_tasks_pending", "index (partial status='pending')"),
    ("idx_plugin_tasks_asset", "index"),
    ("idx_plugin_tasks_failed", "index (partial status='failed')"),
    ("jobs", "table"),
    ("idx_jobs_ready", "index (partial pending)"),
    ("idx_jobs_kind_status", "index"),
    ("uq_jobs_pending_asset", "index (partial pending)"),
    ("uq_jobs_pending_folder", "index (partial, empty)"),
    ("idx_jobs_folder", "index (partial, empty)"),
];

/// 각 객체의 (페이지 바이트, 페이로드 바이트)
fn dbstat(env: &Env) -> Vec<(&'static str, &'static str, u64, u64)> {
    OBJECTS
        .iter()
        .map(|(name, kind)| {
            let (pg, payload): (i64, i64) = env
                .pool
                .read(|c| {
                    Ok(c.query_row("SELECT COALESCE(SUM(pgsize),0), COALESCE(SUM(payload),0) FROM dbstat WHERE name = ?1", [name], |r| Ok((r.get(0)?, r.get(1)?)))?)
                })
                .expect("dbstat (needs SQLITE_ENABLE_DBSTAT_VTAB)");
            (*name, *kind, pg as u64, payload as u64)
        })
        .collect()
}

fn print_sizes(env: &Env, label: &str, rows: u64, providers: u64) {
    env.checkpoint();
    say(format!("  [{label}] after wal_checkpoint(TRUNCATE): db+wal+shm files {:.1} MiB", mib(db_files_bytes(&env.root.join("mam.sqlite")))));
    let (mut tasks_total, mut jobs_total) = (0u64, 0u64);
    for (name, kind, pg, payload) in dbstat(env) {
        say(format!("    {name:<26} {kind:<34} {:>9.2} MiB (payload {:>8.2} MiB)", mib(pg), mib(payload)));
        if name.contains("plugin_tasks") {
            tasks_total += pg;
        } else if name.contains("jobs") {
            jobs_total += pg;
        }
    }
    let per_provider = tasks_total as f64 / providers as f64;
    say(format!(
        "    plugin_tasks table+indexes = {:.2} MiB ({:.1} B/row over {rows} rows x {providers} provider(s)) => {:.2} MiB per provider per 1M rows",
        mib(tasks_total),
        tasks_total as f64 / (rows * providers) as f64,
        mib(per_provider as u64) * 1_000_000.0 / rows as f64
    ));
    say(format!("    jobs table+indexes         = {:.2} MiB ({:.1} B/row over {rows} rows)", mib(jobs_total), jobs_total as f64 / rows as f64));
}

fn wait_tasks(env: &Env, at_least: i64) -> Duration {
    let t = Instant::now();
    while env.count("SELECT COALESCE(MAX(task_id), 0) FROM plugin_tasks") < at_least {
        assert!(t.elapsed() < Duration::from_secs(3600), "run-all did not finish");
        std::thread::sleep(Duration::from_millis(50));
    }
    t.elapsed()
}

/// 같은 SQL·Low 쓰기 경로 배치 루프 — `Core::write_installed` 와 같다 (설치 확인 -> `run_all_batch` -> `enqueue_jobs_tx`, 5,000 행, 배치 사이 50 ms 양보)
fn manual_run_all(env: &Env, plugin: &str) -> (Duration, Lat, Lat) {
    let (mut outer, mut inner) = (Lat::default(), Lat::default());
    let (mut after, t0) = (0i64, Instant::now());
    loop {
        let (id, t) = (plugin.to_owned(), Instant::now());
        let (last, scanned, tx_time) = env
            .pool
            .write(Priority::Low, move |c| {
                with_immediate_tx(c, |tx| {
                    let started = Instant::now();
                    if plugin_settings::get(tx, INSTALLED, &id)?.is_none() {
                        return Ok((None, 0, started.elapsed()));
                    }
                    let kinds: &[&str] = &["image"];
                    let page = run_all_batch(tx, &id, &[(PROVIDER, kinds)], after, RUN_ALL_BATCH, 1_700_000_000_000)?;
                    enqueue_jobs_tx(tx, &page.pending, JobKind::Plugin.default_priority(), 1_700_000_000_000)?;
                    Ok((page.last, page.scanned, started.elapsed()))
                })
            })
            .expect("batch");
        outer.push(t.elapsed());
        inner.push(tx_time);
        match last {
            Some(l) if scanned as i64 >= RUN_ALL_BATCH => after = l,
            _ => break,
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    (t0.elapsed(), outer, inner)
}

pub fn run(scratch: &Path, assets: u64) {
    let base = scratch.join("m1");
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join("MAMData");
    let t = Instant::now();
    synth::build(&root, assets, true, 0);
    say(format!("M1 dataset: {assets} active image assets + {assets} active locations, built in {:.1}s; db {:.1} MiB", t.elapsed().as_secs_f64(), mib(db_files_bytes(&root.join("mam.sqlite")))));
    let env = Env::open(&root, None, None);
    env.install(PLUGIN_A);
    env.install(PLUGIN_B);
    let n = i64::try_from(assets).unwrap_or(i64::MAX);

    // 기준선: run-all 없음
    let (r, w) = (Load::spawn(Duration::from_millis(20), env.reader(assets, 1)), Load::spawn(Duration::from_millis(20), env.editor(assets, 2)));
    std::thread::sleep(Duration::from_secs(8));
    report_pair("idle baseline 8s", r, w);

    // 1차: 실제 PluginJobs::run_all (플러그인 A)
    let (r, w) = (Load::spawn(Duration::from_millis(20), env.reader(assets, 3)), Load::spawn(Duration::from_millis(20), env.editor(assets, 4)));
    let t = Instant::now();
    assert!(env.jobs.run_all(PLUGIN_A, PROVIDER).expect("run_all").started);
    wait_tasks(&env, n);
    let wall_real = t.elapsed();
    report_pair("during REAL PluginJobs::run_all", r, w);
    say(format!("  REAL run_all wall (call -> last batch visible, 50 ms polling) = {:.1}s for {assets} assets ({:.0} assets/s)", wall_real.as_secs_f64(), assets as f64 / wall_real.as_secs_f64()));
    say(format!("  rows: plugin_tasks={} jobs(plugin,pending)={}", env.count("SELECT COUNT(*) FROM plugin_tasks"), env.count("SELECT COUNT(*) FROM jobs WHERE kind='plugin' AND status='pending'")));
    print_sizes(&env, "1 provider", assets, 1);

    // 2차: 배치 루프 (플러그인 B) — 배치 시간
    let (r, w) = (Load::spawn(Duration::from_millis(20), env.reader(assets, 5)), Load::spawn(Duration::from_millis(20), env.editor(assets, 6)));
    let (wall, outer, inner) = manual_run_all(&env, PLUGIN_B);
    report_pair("during manual batch loop (2nd provider)", r, w);
    say(format!("  manual loop wall = {:.1}s (incl. 50 ms yield x {} batches)", wall.as_secs_f64(), outer.0.len()));
    say(outer.line("  per-batch Low write incl. queue wait (5,000 rows)"));
    say(inner.line("  per-batch tx body only"));
    say(format!("  writer occupancy by run-all batches = {:.1}% of wall", 100.0 * inner.sum_ms() / (wall.as_secs_f64() * 1000.0)));
    say(format!("  rows: plugin_tasks={} jobs(plugin)={}", env.count("SELECT COUNT(*) FROM plugin_tasks"), env.count("SELECT COUNT(*) FROM jobs WHERE kind='plugin'")));
    print_sizes(&env, "2 providers", assets, 2);
    say("  (projection for 3 providers: see report; each extra provider adds the plugin_tasks delta between 1 and 2 providers; jobs rows are shared, one per asset)");

    env.shutdown();
    drop(env);
    let _ = std::fs::remove_dir_all(&base);
    say("M1 cleanup: dataset directory deleted");
}
