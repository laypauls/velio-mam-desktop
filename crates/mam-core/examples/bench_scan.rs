//! 스캔 측정 도구 (P2-2) — 합성 폴더 생성, 스캔 시간 측정, 중단 후 이어하기 확인.
//!
//! ```text
//! cargo run --release -p mam-core --example bench_scan -- gen  <dir> <files> [dirs]
//! cargo run --release -p mam-core --example bench_scan -- scan <data-dir> <dir>     # DB 는 <data-dir>/mam.sqlite
//! cargo run --release -p mam-core --example bench_scan -- count <data-dir>
//! ```
//!
//! `scan` 은 실제 볼륨 API·파일 ID·BLAKE3 를 쓰며 DB 가 이미 있으면 이어서(재스캔) 실행한다.
//! 중단 후 이어하기 측정은 `scan` 을 백그라운드로 띄워 프로세스를 강제 종료한 뒤 같은 인자로 다시 실행한다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::scan::{ScanOutcome, scan_folder};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};

fn generate(dir: &Path, files: usize, dirs: usize) -> std::io::Result<()> {
    let per_dir = files.div_ceil(dirs.max(1));
    let mut n = 0;
    for d in 0..dirs {
        let sub = dir.join(format!("album_{d:04}"));
        fs::create_dir_all(&sub)?;
        for _ in 0..per_dir {
            if n == files {
                return Ok(());
            }
            // 내용이 서로 달라야 해시 충돌 후보가 생기지 않는다 (스캔은 내용을 읽지 않는다)
            fs::write(sub.join(format!("img_{n:06}.jpg")), n.to_le_bytes())?;
            n += 1;
        }
    }
    Ok(())
}

fn open_ingest(data: &Path) -> Result<Arc<Ingest>, Box<dyn std::error::Error>> {
    fs::create_dir_all(data)?;
    let db = data.join("mam.sqlite");
    let mut conn = open(&db)?;
    migrate(&mut conn, &data.join("backups"), &SystemClock)?;
    drop(conn);
    Ok(Ingest::new(
        IngestDeps {
            pool: Arc::new(DbPool::open(&db, 2)?),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![data.to_path_buf()],
            ..IngestConfig::default()
        },
    ))
}

fn counts(ingest: &Ingest) -> Result<(i64, i64, String), Box<dyn std::error::Error>> {
    Ok(ingest.env().pool.read(|c| {
        let assets: i64 = c.query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))?;
        let present: i64 = c.query_row(
            "SELECT COUNT(*) FROM file_locations WHERE presence = 'present' AND detached_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let run: String = c
            .query_row(
                "SELECT status || ' cursor=' || IFNULL(cursor, '-') FROM scan_runs ORDER BY run_id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "no run".into());
        Ok((assets, present, run))
    })?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("gen") => {
            let dir = PathBuf::from(arg(1).ok_or("gen <dir> <files> [dirs]")?);
            let files: usize = arg(2).ok_or("files")?.parse()?;
            let dirs: usize = arg(3).map_or(Ok(100), str::parse)?;
            let t = Instant::now();
            generate(&dir, files, dirs)?;
            println!(
                "generated {files} files in {dirs} dirs in {:?}",
                t.elapsed()
            );
        }
        Some("scan") => {
            let data = PathBuf::from(arg(1).ok_or("scan <data-dir> <dir>")?);
            let dir = arg(2).ok_or("dir")?;
            let ingest = open_ingest(&data)?;
            let existing = ingest.list_folders()?;
            let folder_id = match existing.first() {
                Some(f) => f.folder_id,
                None => ingest.add_folder(dir)?.folder_id,
            };
            let t = Instant::now();
            let outcome = scan_folder(ingest.env(), folder_id, &|| false, &AtomicBool::new(false))?;
            let elapsed = t.elapsed();
            let (assets, present, run) = counts(&ingest)?;
            match outcome {
                ScanOutcome::Completed { counts } => println!(
                    "completed in {elapsed:?}: processed={} created={} unchanged={} updated={} moved={} missing={} errors={}",
                    counts.processed,
                    counts.created,
                    counts.unchanged,
                    counts.updated,
                    counts.moved,
                    counts.missing,
                    counts.errors
                ),
                other => println!("{other:?} in {elapsed:?}"),
            }
            println!("assets={assets} present_locations={present} last_run={run}");
            ingest.env().pool.shutdown();
        }
        Some("count") => {
            let ingest = open_ingest(&PathBuf::from(arg(1).ok_or("count <data-dir>")?))?;
            let (assets, present, run) = counts(&ingest)?;
            println!("assets={assets} present_locations={present} last_run={run}");
            ingest.env().pool.shutdown();
        }
        _ => println!("usage: gen | scan | count (see file header)"),
    }
    Ok(())
}
