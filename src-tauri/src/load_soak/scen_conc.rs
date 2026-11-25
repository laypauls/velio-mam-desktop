//! P4 후속 2 — 처리 동시성(메타·썸네일 작업 수, ffmpeg 스레드)별 처리량·CPU·검색 지연.
//!
//! 실제 합성 미디어 파일(JPEG/PNG/WAV/mp4 혼합)을 새 관리 폴더로 등록해 스캔·메타·썸네일·해시를 끝까지 돌리면서,
//! 같은 프로세스에서 검색 질의를 계속 던져 부하 중 지연을 잰다. AI 작업은 돌리지 않는다.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::bench_env::{Bench, Options};
use super::media_gen::{Templates, generate};
use super::metrics::{logical_cpus, mb, proc_now};
use super::queries::{Ctx, run_fixed, run_until};
use super::report::{print_conditions, wait_for};
use super::scen_m12::{build_synth_library, derivative_done, wait_ready};

/// 한 설정: (메타 동시성, 썸네일 동시성, ffmpeg -threads)
#[derive(Clone, Copy)]
pub struct Config {
    pub meta: usize,
    pub thumb: usize,
    pub ffmpeg_threads: u32,
}

pub struct Scale {
    pub base_assets: usize,
    pub files: usize,
    pub light: bool,
}

pub fn run(root: &Path, s: &Scale, configs: &[Config]) {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root).expect("root");
    print_conditions(
        "P4F-2 조건",
        &[
            (
                "기준 라이브러리",
                format!("{} 합성 자산 (검색 대상)", s.base_assets),
            ),
            (
                "신규 실제 파일",
                format!(
                    "{} (JPEG 640x480 60% / 1280x720 35% / 2400x1600 5%, PNG, WAV, 2초 mp4 혼합)",
                    s.files
                ),
            ),
            ("해시 동시성", "1 (기본값 유지)".into()),
        ],
    );
    let templates = Templates::build(&root.join("scratch"), s.light);
    let files_dir = root.join("files");
    let made = generate(&files_dir, &templates, 0..s.files as u64, 31);
    println!(
        "- 파일 생성 {} 개 {:.2} GB (영상 {}, 음성 {}) {:.0}s",
        made.files,
        made.bytes as f64 / 1e9,
        made.videos,
        made.audios,
        made.secs
    );
    println!(
        "\n| 메타/썸네일/ffmpeg | 소요 s | 파일/s | 메타 | 썸네일 | 시스템 CPU % | 프로세스 CPU % (논리 CPU 환산) | 최대 RSS MB | 검색 n | 검색 P50 ms | P95 ms | P99 ms | max ms | 유휴 P95 ms |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"
    );
    for cfg in configs {
        run_one(root, s, &files_dir, *cfg);
    }
}

fn run_one(root: &Path, s: &Scale, files_dir: &Path, cfg: Config) {
    let dir = root.join(format!(
        "m{}t{}f{}",
        cfg.meta, cfg.thumb, cfg.ffmpeg_threads
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cfg dir");
    let lib = build_synth_library(&dir, s.base_assets, 1000);
    let opts = Options {
        meta: Some(cfg.meta),
        thumb: Some(cfg.thumb),
        ffmpeg_threads: Some(cfg.ffmpeg_threads),
        ..Options::default()
    };
    let bench = Bench::start(&dir, &opts);
    if wait_ready(&bench, s.base_assets as i64, Duration::from_secs(1800)).is_none() {
        println!(
            "| {}/{}/{} | 색인 준비 실패 |",
            cfg.meta, cfg.thumb, cfg.ffmpeg_threads
        );
        bench.shutdown();
        return;
    }
    let ctx = Ctx {
        folder_id: lib.folder_id,
    };
    let idle = run_fixed(&bench.search, &ctx, 10, 2, 42).all();
    std::thread::sleep(Duration::from_secs(5)); // 준비 직후 잔여 작업이 가라앉게

    let stop = Arc::new(AtomicBool::new(false));
    let q = {
        let (b, stop) = (Arc::clone(&bench), Arc::clone(&stop));
        let folder_id = lib.folder_id;
        std::thread::spawn(move || {
            run_until(
                &b.search,
                &Ctx { folder_id },
                &stop,
                Duration::from_millis(20),
                43,
            )
        })
    };
    let floor = s.base_assets as i64;
    let (p0, t0) = (proc_now(), Instant::now());
    bench
        .ingest
        .add_folder(&files_dir.to_string_lossy())
        .expect("add_folder");
    let want = s.files as i64;
    let done = wait_for(Duration::from_secs(3600), || {
        bench.count("SELECT count(*) FROM media_assets") >= floor + want
            && bench.jobs_busy() == (0, 0)
    });
    let secs = t0.elapsed().as_secs_f64();
    let p1 = proc_now();
    stop.store(true, Ordering::Relaxed);
    let rep = q.join().expect("query thread");
    let m = rep.all();
    let cpus = logical_cpus() as f64;
    let sys = 100.0
        * (1.0
            - p1.sys_idle.saturating_sub(p0.sys_idle) as f64
                / p1.sys_total.saturating_sub(p0.sys_total).max(1) as f64);
    let procc = 100.0 * (p1.cpu_100ns.saturating_sub(p0.cpu_100ns)) as f64 / 1e7 / secs / cpus;
    let (meta, thumb) = (
        derivative_done(&bench, "meta", floor),
        derivative_done(&bench, "thumb", floor),
    );
    let failed = bench.count("SELECT count(*) FROM jobs WHERE status = 'failed'");
    println!(
        "| {}/{}/{} | {secs:.0}{} | {:.1} | {meta} | {thumb} | {sys:.0} | {procc:.0} | {:.0} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |{}",
        cfg.meta,
        cfg.thumb,
        cfg.ffmpeg_threads,
        if done { "" } else { " (시간 초과)" },
        want as f64 / secs,
        mb(p1.peak_rss),
        m.n,
        m.p50,
        m.p95,
        m.p99,
        m.max,
        idle.p95,
        if failed > 0 {
            format!(" 실패 작업 {failed}")
        } else {
            String::new()
        }
    );
    bench.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
