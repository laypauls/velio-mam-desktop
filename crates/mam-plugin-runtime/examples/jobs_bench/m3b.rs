//! M3(b) — 실제 파일: 작은 JPEG/PNG N 개를 만들고(`gen`), 실제 수집(스캔 -> 메타 -> 썸네일 -> 해시, `Ingest` + `ProcessingEnv`)을
//! 플러그인 없이(`noplugin`) / 실제 `PluginRuntime`(실제 워커)에 이미지 provider 1개를 켜고(`plugin`) 돌려 A/B 비교한다.
//! `clean` 은 파일 폴더를 지운다. 이미지만 — 동영상·음성(FFmpeg)·WebView/UI 는 측정하지 않는다.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::{ImageFormat, Rgb, RgbImage};
use mam_core::Blake3Hasher;
use mam_core::WindowsFileIdentity;
use mam_core::WindowsVolumes;
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::jobs::JobKind;
use mam_core::processing::ProcessingEnv;
use mam_db::{DbPool, Priority, SystemClock};
use mam_media::{MediaTools, ProcessRunner, ToolConfig};

use crate::env::{Env, PLUGIN_A, worker_from_env};
use crate::synth;
use crate::util::{Load, dir_size, mib, say};

/// 결정적·파일마다 다른 내용의 작은 이미지 (256x192)
fn picture(n: u64) -> RgbImage {
    RgbImage::from_fn(256, 192, |x, y| {
        let (x, y) = (u64::from(x), u64::from(y));
        let v = (x * 3 + y * 5 + n * 7) ^ ((x * y + n) >> 3);
        Rgb([(v & 0xff) as u8, ((v >> 2) as u8).wrapping_add(x as u8), (n as u8).wrapping_add(y as u8)])
    })
}

pub fn generate(dir: &Path, files: usize) {
    let t = Instant::now();
    let workers = 6usize;
    std::thread::scope(|s| {
        for w in 0..workers {
            s.spawn(move || {
                for n in (w..files).step_by(workers) {
                    let sub = dir.join(format!("album_{:04}", n / 100));
                    let _ = std::fs::create_dir_all(&sub);
                    let (ext, fmt) = if n % 5 == 4 { ("png", ImageFormat::Png) } else { ("jpg", ImageFormat::Jpeg) };
                    let path = sub.join(format!("f_{n:06}.{ext}"));
                    if path.exists() {
                        continue;
                    }
                    let mut out = std::io::Cursor::new(Vec::new());
                    picture(n as u64).write_to(&mut out, fmt).expect("encode");
                    std::fs::write(&path, out.into_inner()).expect("write image");
                }
            });
        }
    });
    say(format!("M3b gen: {files} images (80% JPEG / 20% PNG, 256x192) in {:.1}s; {:.1} MiB on disk (sum of file sizes)", t.elapsed().as_secs_f64(), mib(dir_size(dir))));
}

fn pending_by_kind(pool: &DbPool) -> std::collections::BTreeMap<String, i64> {
    pool.read(|c| {
        let mut st = c.prepare("SELECT kind, COUNT(*) FROM jobs WHERE status IN ('pending','running') GROUP BY kind")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
    .expect("jobs by kind")
}

fn busy(pool: &DbPool, kinds: &[&str]) -> bool {
    let m = pending_by_kind(pool);
    kinds.iter().any(|k| m.get(*k).copied().unwrap_or(0) > 0)
}

fn count(pool: &DbPool, sql: &'static str) -> i64 {
    pool.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).expect("count")
}

/// `FeedOnly` = 플러그인 설치 + 변경 피드는 돌지만 `plugin` 실행기를 꽂지 않는다 (피드 비용과 실행 비용을 가르는 보조 측정)
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    NoPlugin,
    Plugin,
    FeedOnly,
}

pub fn run(scratch: &Path, files: usize, mode: Mode) {
    let plugin = mode != Mode::NoPlugin;
    let base = scratch.join("m3b");
    let dir = base.join("files");
    let run_dir = base.join(match mode { Mode::NoPlugin => "run_noplugin", Mode::Plugin => "run_plugin", Mode::FeedOnly => "run_feedonly" });
    let _ = std::fs::remove_dir_all(&run_dir);
    let data = run_dir.join("MAMData");
    let db = synth::empty_db(&data);
    let pool = Arc::new(DbPool::open(&db, 4).expect("pool"));
    let ingest = Ingest::new(
        IngestDeps { pool: Arc::clone(&pool), clock: Arc::new(SystemClock), volumes: Arc::new(WindowsVolumes), identity: Arc::new(WindowsFileIdentity), hasher: Arc::new(Blake3Hasher) },
        IngestConfig { auto_excludes: vec![data.clone()], ..IngestConfig::default() },
    );
    let tools = Arc::new(MediaTools::discover(&ToolConfig::default(), Arc::new(ProcessRunner)));
    ingest.enable_processing(ProcessingEnv::new(Arc::clone(&pool), Arc::new(SystemClock), tools, Arc::new(Blake3Hasher), data.join("cache")));
    let env = plugin.then(|| {
        let worker = worker_from_env().expect("set MAM_PLUGIN_WORKER_EXE for the plugin run");
        let env = Env::with_pool(Arc::clone(&pool), &data, Some(worker), Some(Arc::clone(ingest.jobs())));
        env.install(PLUGIN_A);
        if mode == Mode::Plugin {
            ingest.register_executor(JobKind::Plugin, env.jobs.executor());
        }
        env
    });
    say(format!("M3b run: {} — {files} real images from {}", match mode { Mode::NoPlugin => "no plugin", Mode::Plugin => "1 enabled trivial provider on the REAL worker", Mode::FeedOnly => "plugin installed + feed running, NO plugin executor registered (feed cost only)" }, dir.display()));
    ingest.add_folder(&dir.to_string_lossy()).expect("add folder");
    let t0 = Instant::now();
    ingest.start().expect("ingest start");
    if let Some(e) = &env {
        e.jobs.start_feed();
    }

    // 사용자 태그 편집(High) 100 ms 간격 — 자산 1..=1000 이 생긴 뒤 시작. Low 쓰기 프로브 50 ms 간격(큐 대기 + 실행)
    let target = i64::try_from(files).unwrap_or(i64::MAX);
    let wait_assets = |n: i64| {
        while count(&pool, "SELECT COUNT(*) FROM media_assets") < n {
            assert!(t0.elapsed() < Duration::from_secs(7200), "scan too slow");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    wait_assets(1000.min(target));
    let ud = ingest_userdata(&pool);
    let high = Load::spawn(Duration::from_millis(100), {
        let mut k = 0u64;
        move || {
            k += 1;
            let a = i64::try_from(1 + k % 1000).unwrap_or(1);
            ud.update_asset_tags(mam_core::userdata::UpdateTagsInput { asset_ids: vec![a], add: vec![format!("user-{}", k % 32)], remove: vec![] }).is_ok()
        }
    });
    let p2 = Arc::clone(&pool);
    let low = Load::spawn(Duration::from_millis(50), move || {
        p2.write(Priority::Low, |c| Ok(c.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('bench_probe', '1')", [])?)).is_ok()
    });

    wait_assets(target);
    let t_scanned = t0.elapsed();
    while busy(&pool, &["scan", "meta", "thumb", "hash"]) {
        assert!(t0.elapsed() < Duration::from_secs(7200), "pipeline too slow");
        std::thread::sleep(Duration::from_millis(250));
    }
    let t_pipeline = t0.elapsed();
    let (high_lat, high_fail) = high.finish();
    let (low_lat, low_fail) = low.finish();
    say(format!("  assets in DB {} ; all files discovered at {:.1}s", count(&pool, "SELECT COUNT(*) FROM media_assets"), t_scanned.as_secs_f64()));
    say(format!("  scan+meta+thumb+hash drained at {:.1}s => {:.1} assets/s", t_pipeline.as_secs_f64(), files as f64 / t_pipeline.as_secs_f64()));
    say(high_lat.line(&format!("  High write: user tag edit, real update_asset_tags, 100 ms pace (fail {high_fail})")));
    say(low_lat.line(&format!("  Low write probe (settings upsert), 50 ms pace, queue wait + exec (fail {low_fail})")));
    say(format!("  thumbnails/derivatives: cache dir {:.1} MiB; db {:.1} MiB", mib(dir_size(&data.join("cache"))), mib(crate::util::db_files_bytes(&db))));

    if let Some(e) = &env {
        // 부하를 멈춘 뒤 플러그인 쪽이 따라잡는 시간 (피드 -> 작업 -> 실제 워커 호출)
        let t = Instant::now();
        // FeedOnly 는 실행기가 없어 대기 작업이 쌓이기만 하고 피드가 PENDING_CAP 에서 멈춘다 — 기다리지 않는다
        while mode == Mode::Plugin && (busy(&pool, &["plugin"]) || count(&pool, "SELECT COUNT(*) FROM plugin_tasks WHERE status = 'pending'") > 0 || feed_behind(&pool)) {
            assert!(t.elapsed() < Duration::from_secs(7200), "plugin too slow");
            std::thread::sleep(Duration::from_millis(250));
        }
        say(format!("  plugin drained {:.1}s after the pipeline ({:.1}s total, {:.1} assets/s end-to-end incl. plugin)", t.elapsed().as_secs_f64(), (t_pipeline + t.elapsed()).as_secs_f64(), files as f64 / (t_pipeline + t.elapsed()).as_secs_f64()));
        say(format!("  plugin_tasks: done={} failed={} pending={}; plugin_kv rows={}", count(&pool, "SELECT COUNT(*) FROM plugin_tasks WHERE status='done'"), count(&pool, "SELECT COUNT(*) FROM plugin_tasks WHERE status='failed'"), count(&pool, "SELECT COUNT(*) FROM plugin_tasks WHERE status='pending'"), count(&pool, "SELECT COUNT(*) FROM plugin_kv")));
        e.shutdown();
    }
    ingest.shutdown();
    drop(env);
    drop(ingest);
    drop(pool);
    let _ = std::fs::remove_dir_all(&run_dir);
    say("M3b run cleanup: run directory (db + thumbnails) deleted");
}

fn feed_behind(pool: &DbPool) -> bool {
    pool.read(|c| Ok(mam_db::plugin_tasks::feed_cursor(c, PLUGIN_A)?.unwrap_or(0) < mam_db::plugin_tasks::max_event_id(c)?)).unwrap_or(true)
}

fn ingest_userdata(pool: &Arc<DbPool>) -> Arc<mam_core::userdata::UserData> {
    Arc::new(mam_core::userdata::UserData::new(Arc::clone(pool), Arc::new(SystemClock)))
}

pub fn clean(scratch: &Path) -> PathBuf {
    let dir = scratch.join("m3b");
    let _ = std::fs::remove_dir_all(&dir);
    say(format!("M3b clean: {} deleted", dir.display()));
    dir
}
