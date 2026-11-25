//! 자산 처리 측정 도구 (P2-3) — 합성 혼합 샘플 생성, 스캔 -> 메타 -> 썸네일 -> 해시 전체 처리 시간 측정.
//!
//! ```text
//! cargo run --release -p mam-core --example bench_processing -- gen <dir> <files>
//! cargo run --release -p mam-core --example bench_processing -- run <data-dir> <dir>
//! ```
//!
//! `gen` 은 이미지 70%(JPEG 소형·대형, PNG) / 영상 20%(작은 mp4·webm 사본 + 파일마다 다른 꼬리 바이트) / 음성 10%(WAV) 를 만든다.
//! 영상은 실제로는 수 KB 짜리 클립이라 대형 영상 디코딩 비용은 반영하지 못한다. `run` 은 실제 볼륨 API·파일 ID·BLAKE3·
//! PATH 의 ffmpeg/ffprobe 를 쓰고, 단계별 완료 시각을 출력한다. 결과 해석은 작업 문서의 측정 절을 본다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::{ImageFormat, Rgb, RgbImage};
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::processing::ProcessingEnv;
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};
use mam_media::{MediaTools, ProcessRunner, ToolConfig};

/// 파일마다 다른 무작위 비슷한 값 (LCG) - 압축이 쉬운 단색 이미지를 피한다
fn noise_image(w: u32, h: u32, seed: u64) -> RgbImage {
    let mut s = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    RgbImage::from_fn(w, h, |x, y| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let n = (s >> 60) as u8;
        Rgb([
            ((x * 255 / w) as u8).wrapping_add(n),
            ((y * 255 / h) as u8).wrapping_add(n),
            (((x + y) * 255 / (w + h)) as u8).wrapping_add(n),
        ])
    })
}

fn wav(seed: u32) -> Vec<u8> {
    let data: Vec<u8> = (0..16000u32)
        .map(|i| ((i * 7 + seed) % 251) as u8)
        .collect();
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    for x in [16u32, 0x0001_0001, 8000, 8000, 0x0008_0001] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

fn generate(dir: &Path, files: usize) -> Result<(), Box<dyn std::error::Error>> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../mam-media/tests/fixtures");
    let mp4 = fs::read(fixtures.join("tiny.mp4"))?;
    let webm = fs::read(fixtures.join("tiny.webm"))?;
    for n in 0..files {
        let sub = dir.join(format!("album_{:03}", n / 50));
        fs::create_dir_all(&sub)?;
        let path = |ext: &str| sub.join(format!("f_{n:05}.{ext}"));
        match n % 10 {
            0..=3 => {
                let mut out = std::io::Cursor::new(Vec::new());
                noise_image(1280, 960, n as u64).write_to(&mut out, ImageFormat::Jpeg)?;
                fs::write(path("jpg"), out.into_inner())?;
            }
            4 => {
                let mut out = std::io::Cursor::new(Vec::new());
                noise_image(4000, 3000, n as u64).write_to(&mut out, ImageFormat::Jpeg)?;
                fs::write(path("jpg"), out.into_inner())?;
            }
            5 | 6 => {
                let mut out = std::io::Cursor::new(Vec::new());
                noise_image(800, 600, n as u64).write_to(&mut out, ImageFormat::Png)?;
                fs::write(path("png"), out.into_inner())?;
            }
            7 | 8 => {
                // 끝에 파일마다 다른 바이트를 붙여 해시를 다르게 한다 (컨테이너는 꼬리 바이트를 무시한다)
                let (mut bytes, ext) = if n % 20 < 10 {
                    (mp4.clone(), "mp4")
                } else {
                    (webm.clone(), "webm")
                };
                bytes.extend_from_slice(&(n as u64).to_le_bytes());
                fs::write(path(ext), bytes)?;
            }
            _ => fs::write(path("wav"), wav(n as u32))?,
        }
    }
    Ok(())
}

fn pending_by_kind(ingest: &Ingest) -> Result<[i64; 4], Box<dyn std::error::Error>> {
    Ok(ingest.env().pool.read(|c| {
        let mut out = [0i64; 4];
        let mut stmt = c.prepare(
            "SELECT kind, COUNT(*) FROM jobs WHERE status IN ('pending','running')
             AND kind IN ('scan','meta','thumb','hash') GROUP BY kind",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (kind, n) = row?;
            match kind.as_str() {
                "scan" => out[0] = n,
                "meta" => out[1] = n,
                "thumb" => out[2] = n,
                _ => out[3] = n,
            }
        }
        Ok(out)
    })?)
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            total += if p.is_dir() {
                dir_size(&p)
            } else {
                e.metadata().map_or(0, |m| m.len())
            };
        }
    }
    total
}

fn run(data: &Path, dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(data)?;
    let db = data.join("mam.sqlite");
    let mut conn = open(&db)?;
    migrate(&mut conn, &data.join("backups"), &SystemClock)?;
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 4)?);
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&pool),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![data.to_path_buf()],
            ..IngestConfig::default()
        },
    );
    let tools = Arc::new(MediaTools::discover(
        &ToolConfig::default(),
        Arc::new(ProcessRunner),
    ));
    println!(
        "ffmpeg={} ffprobe={} cpus={}",
        tools.has_ffmpeg(),
        tools.has_ffprobe(),
        std::thread::available_parallelism().map_or(0, usize::from)
    );
    let cache = data.join("cache");
    ingest.enable_processing(ProcessingEnv::new(
        Arc::clone(&pool),
        Arc::new(SystemClock),
        tools,
        Arc::new(Blake3Hasher),
        cache.clone(),
    ));
    ingest.add_folder(dir)?;
    let t0 = Instant::now();
    ingest.start()?;

    let names = ["scan", "meta", "thumb", "hash"];
    let mut done_at: [Option<Duration>; 4] = [None; 4];
    let mut seen_jobs = false;
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let pending = pending_by_kind(&ingest)?;
        seen_jobs |= pending.iter().any(|n| *n > 0);
        for (i, n) in pending.iter().enumerate() {
            // 스캔이 끝난 뒤(= 처리 작업이 등록된 뒤)부터 각 종류의 완료를 본다
            if seen_jobs && *n == 0 && done_at[i].is_none() && (i == 0 || done_at[0].is_some()) {
                done_at[i] = Some(t0.elapsed());
            }
        }
        if done_at.iter().all(Option::is_some) {
            break;
        }
        if t0.elapsed() > Duration::from_secs(1800) {
            println!("timeout; pending={pending:?}");
            break;
        }
    }
    for (name, at) in names.iter().zip(done_at) {
        println!(
            "{name:>6} drained at {:>8.2?}",
            at.unwrap_or_else(|| t0.elapsed())
        );
    }
    let (assets, meta_done, thumb_done, strip_done, thumb_failed, hashed): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = pool.read(|c| {
        let q = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok((
            q("SELECT COUNT(*) FROM media_assets")?,
            q("SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'meta' AND state = 'done'")?,
            q("SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'thumb' AND state = 'done'")?,
            q("SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'strip' AND state = 'done'")?,
            q("SELECT COUNT(*) FROM asset_derivatives WHERE state = 'failed'")?,
            q("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'done'")?,
        ))
    })?;
    println!(
        "assets={assets} meta_done={meta_done} thumb_done={thumb_done} strip_done={strip_done} failed={thumb_failed} hashed={hashed}"
    );
    println!("cache bytes={}", dir_size(&cache));
    ingest.shutdown();
    pool.shutdown();
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("gen") => {
            let dir = PathBuf::from(arg(1).ok_or("gen <dir> <files>")?);
            let files: usize = arg(2).ok_or("files")?.parse()?;
            let t = Instant::now();
            generate(&dir, files)?;
            println!("generated {files} files in {:?}", t.elapsed());
        }
        Some("run") => run(
            Path::new(arg(1).ok_or("run <data-dir> <dir>")?),
            arg(2).ok_or("dir")?,
        )?,
        _ => println!("usage: gen | run (see file header)"),
    }
    Ok(())
}
