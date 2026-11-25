//! `polling` 감지기 PoC 측정 (멀티 스캔 0단계 a·b) — 실제 `Snapshot`/`StdFs` 로 디렉터리 가지치기 폴링 비용을 잰다.
//!
//! ```text
//! cargo run --release -p mam-core --example poc_polling -- gen  <dir> <top> <mid> <leaf> <files-per-leaf>   # 빈 파일 합성 트리
//! cargo run --release -p mam-core --example poc_polling -- list <dir> <runs>    # 전체 목록(저장 없음) 대 기준선(스냅숏 저장) 비용 + 스냅숏 힙
//! cargo run --release -p mam-core --example poc_polling -- prune <dir> <runs> <complete-tops>   # 가지치기 주기: 변경 0 / 10 / 1000 개 디렉터리
//! cargo run --release -p mam-core --example poc_polling -- dirmtime <scratch>   # NTFS 디렉터리 수정 시각 갱신 규칙
//! ```
//!
//! 합성 데이터는 측정 뒤 지운다(재생성 가능). `<dir>` 은 사용자 폴더가 아니라 측정용 폴더여야 한다.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mam_core::watch::detectors::polling::{Change, PollFs, Snapshot, StdFs};

/// 힙 사용량 계수기 — 스냅숏이 실제로 쥔 바이트를 재기 위한 것
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LIVE.fetch_add(n, Ordering::Relaxed);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[(((v.len() - 1) as f64) * p).round() as usize]
}

fn report(label: &str, ms: &mut [f64]) {
    println!(
        "| {label} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
        ms.len(),
        pct(ms, 0.5),
        pct(ms, 0.95),
        pct(ms, 0.99),
        ms.iter().copied().fold(0.0, f64::max)
    );
}

fn generate(dir: &Path, top: usize, mid: usize, leaf: usize, files: usize) -> std::io::Result<()> {
    let threads = 6.min(top);
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|w| {
                s.spawn(move || -> std::io::Result<()> {
                    for t in (w..top).step_by(threads) {
                        for m in 0..mid {
                            for l in 0..leaf {
                                let d = dir
                                    .join(format!("t{t:02}"))
                                    .join(format!("m{m:02}"))
                                    .join(format!("l{l:03}"));
                                fs::create_dir_all(&d)?;
                                for f in 0..files {
                                    File::create(d.join(format!("IMG_{f:06}.jpg")))?;
                                }
                            }
                        }
                    }
                    Ok(())
                })
            })
            .collect();
        hs.into_iter()
            .try_for_each(|h| h.join().expect("gen thread"))
    })
}

/// 저장 없이 전체 목록만 읽는다 (기존 방식의 한 주기 비용 — watch_folder 의 "전체 다시 목록화")
fn plain_listing(root: &Path) -> (usize, usize) {
    let (mut dirs, mut entries) = (0, 0);
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        dirs += 1;
        if let Ok(l) = StdFs.list_dir(&d) {
            entries += l.files.len() + l.dirs.len();
            stack.extend(l.dirs.iter().map(|n| d.join(n)));
        }
    }
    (dirs, entries)
}

fn leaf_path(root: &Path, k: usize, mid: usize, leaf: usize) -> PathBuf {
    let (t, rest) = (k / (mid * leaf), k % (mid * leaf));
    root.join(format!("t{t:02}"))
        .join(format!("m{:02}", rest / leaf))
        .join(format!("l{:03}", rest % leaf))
}

fn cmd_list(root: &Path, runs: usize) {
    println!("| pass | runs | P50 ms | P95 ms | P99 ms | max ms |\n|---|---|---|---|---|---|");
    let (mut plain, mut base) = (Vec::new(), Vec::new());
    let mut mem = (0usize, 0usize, 0usize, 0usize);
    for r in 0..runs {
        let t = Instant::now();
        let (d, e) = plain_listing(root);
        plain.push(t.elapsed().as_secs_f64() * 1000.0);
        let before = LIVE.load(Ordering::Relaxed);
        let t = Instant::now();
        let (snap, stats) = Snapshot::baseline(&StdFs, root, now_ms(), &|| false);
        base.push(t.elapsed().as_secs_f64() * 1000.0);
        let snap = snap.expect("baseline");
        if r == 0 {
            mem = (
                d,
                e,
                LIVE.load(Ordering::Relaxed) - before,
                snap.approx_heap_bytes(),
            );
            println!(
                "<!-- dirs={d} entries={e} stats={} lists={} -->",
                stats.stats, stats.lists
            );
        }
    }
    report("full listing (no store)", &mut plain);
    report("baseline (build snapshot)", &mut base);
    println!(
        "\nsnapshot: dirs={} entries={} measured heap={:.1} MiB (allocator), estimate={:.1} MiB",
        mem.0,
        mem.1,
        mem.2 as f64 / 1_048_576.0,
        mem.3 as f64 / 1_048_576.0
    );
}

fn cmd_prune(root: &Path, runs: usize, tops: usize) {
    let (mid, leaf) = (10, 100);
    let total_leaves = tops * mid * leaf; // 변경은 완전히 생성된 상위 폴더 안의 잎 디렉터리에만 만든다
    let (snap, _) = Snapshot::baseline(&StdFs, root, now_ms(), &|| false);
    let mut snap = snap.expect("baseline");
    println!("dirs={} entries={}", snap.dir_count(), snap.entry_count());
    println!(
        "| changed dirs | runs | P50 ms | P95 ms | P99 ms | max ms |\n|---|---|---|---|---|---|"
    );
    for n in [0usize, 10, 1000] {
        let mut ms = Vec::new();
        let mut verify_ms = Vec::new();
        let mut counts = (0, 0);
        for r in 0..runs {
            // 직전 변경이 "방금" 창(3초)을 벗어나 확인용 재읽기가 끝난 상태에서 시작한다
            std::thread::sleep(Duration::from_millis(3300));
            let _ = snap.tick(&StdFs, root, now_ms(), &|| false);
            let _ = snap.tick(&StdFs, root, now_ms(), &|| false);
            for i in 0..n {
                let d = leaf_path(root, (i * 997 + r * 31) % total_leaves, mid, leaf);
                File::create(d.join(format!("chg_{n}_{r}_{i}.jpg"))).expect("mutate");
            }
            let t = Instant::now();
            let out = snap.tick(&StdFs, root, now_ms(), &|| false);
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
            let paths = out
                .changes
                .iter()
                .filter(|c| matches!(c, Change::Path(_)))
                .count();
            counts = (paths, out.stats.lists);
            assert_eq!(paths, n, "every created file is reported exactly once");
            let t = Instant::now();
            let _ = snap.tick(&StdFs, root, now_ms(), &|| false); // 확인용 재읽기 주기
            verify_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        report(
            &format!("{n} (paths={}, listed dirs={})", counts.0, counts.1),
            &mut ms,
        );
        report(&format!("{n} — following verify tick"), &mut verify_ms);
    }
}

/// NTFS 에서 디렉터리 수정 시각이 언제 바뀌는지 — 폴링 가정의 근거 (b)
fn cmd_dirmtime(scratch: &Path) {
    let d = scratch.join("d");
    fs::create_dir_all(d.join("sub")).expect("mkdir");
    fs::write(d.join("file.bin"), b"0123456789").expect("seed");
    let mt = |p: &Path| StdFs.dir_mtime(p).expect("mtime");
    let step = |name: &str, f: &dyn Fn()| {
        std::thread::sleep(Duration::from_millis(60));
        let (a, b0) = (mt(&d), mt(&d.join("sub")));
        f();
        let (b, b1) = (mt(&d), mt(&d.join("sub")));
        println!(
            "| {name} | {} | {} |",
            if a != b { "changed" } else { "unchanged" },
            if b0 != b1 { "changed" } else { "unchanged" }
        );
    };
    println!("| operation | parent dir mtime | sub dir mtime |\n|---|---|---|");
    step("create child file in dir", &|| {
        fs::write(d.join("new.bin"), b"x").expect("create")
    });
    step("delete child file in dir", &|| {
        fs::remove_file(d.join("new.bin")).expect("rm")
    });
    step("rename child file in dir", &|| {
        fs::rename(d.join("file.bin"), d.join("renamed.bin")).expect("mv")
    });
    step("in-place append to child file", &|| {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(d.join("renamed.bin"))
            .expect("open");
        f.write_all(b"more").expect("write");
        f.sync_all().expect("sync");
    });
    step("in-place overwrite same size", &|| {
        use std::io::{Seek, Write};
        let mut f = fs::OpenOptions::new()
            .write(true)
            .open(d.join("renamed.bin"))
            .expect("open");
        f.seek(std::io::SeekFrom::Start(0)).expect("seek");
        f.write_all(b"ZZ").expect("write");
        f.sync_all().expect("sync");
    });
    step("truncate child file (set_len)", &|| {
        fs::OpenOptions::new()
            .write(true)
            .open(d.join("renamed.bin"))
            .expect("open")
            .set_len(3)
            .expect("len")
    });
    step("set child readonly attribute", &|| {
        let mut p = fs::metadata(d.join("renamed.bin"))
            .expect("meta")
            .permissions();
        p.set_readonly(true);
        fs::set_permissions(d.join("renamed.bin"), p).expect("perm");
    });
    step("create file inside sub (grandchild of dir)", &|| {
        fs::write(d.join("sub").join("g.bin"), b"x").expect("create")
    });
    step("create subdirectory in dir", &|| {
        fs::create_dir(d.join("sub2")).expect("mkdir")
    });
    step("remove subdirectory from dir", &|| {
        fs::remove_dir(d.join("sub2")).expect("rmdir")
    });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a = |i: usize| args.get(i).map(String::as_str);
    let num = |i: usize| {
        a(i).and_then(|s| s.parse::<usize>().ok())
            .expect("number argument")
    };
    match a(0) {
        Some("gen") => {
            let t = Instant::now();
            generate(
                Path::new(a(1).expect("dir")),
                num(2),
                num(3),
                num(4),
                num(5),
            )
            .expect("gen");
            println!("generated in {:?}", t.elapsed());
        }
        Some("list") => cmd_list(Path::new(a(1).expect("dir")), num(2)),
        Some("prune") => cmd_prune(Path::new(a(1).expect("dir")), num(2), num(3)),
        Some("dirmtime") => cmd_dirmtime(Path::new(a(1).expect("dir"))),
        _ => println!("usage: gen | list | prune | dirmtime (see file header)"),
    }
}
