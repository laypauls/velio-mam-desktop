//! P1-4 해시 I/O 측정 — 크기별 파일 묶음을 생성해 BLAKE3 스트리밍 해시 처리량(MB/s)을 잰다.
//!
//! 실행 예 (총 약 11.4 GiB 를 쓰고, 끝나면 지운다):
//!   cargo run --release -p mam-core --example bench_hash -- --dir target/bench-p1-4
//! 옵션: `--huge-gib N`(기본 8, 0 이면 생략) `--keep`(파일 유지)
//!
//! 단계 (묶음마다): ① 첫 해시 패스(생성 직후 — OS 캐시 상태는 통제하지 못함) ② 두 번째 패스(캐시 가열)
//! ③ 캐시 우회 순수 읽기(FILE_FLAG_NO_BUFFERING, 디스크 한계) ④ 메모리 BLAKE3 단일 스레드 한계(CPU).
//! 캐시를 강제로 비우는 방법(관리자 권한 필요)은 쓰지 않았다 — 결과 해석은 작업 문서의 조건 표를 본다.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mam_core::{Blake3Hasher, ContentHasher};

const MIB: u64 = 1024 * 1024;
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;

struct Class {
    name: &'static str,
    size: u64,
    count: usize,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn generate(dir: &Path, class: &Class) -> std::io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    let mut block = vec![0u8; MIB as usize];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15 ^ class.size;
    for chunk in block.chunks_mut(8) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let b = state.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    let mut paths = Vec::new();
    for i in 0..class.count {
        let p = dir.join(format!("{}-{i:05}.bin", class.name));
        let mut f = File::create(&p)?;
        let mut left = class.size;
        let mut n = 0u64;
        while left > 0 {
            block[..8].copy_from_slice(&(i as u64 * 1_000_003 + n).to_le_bytes());
            let take = left.min(MIB) as usize;
            f.write_all(&block[..take])?;
            left -= take as u64;
            n += 1;
        }
        f.sync_all()?;
        paths.push(p);
    }
    Ok(paths)
}

fn hash_pass(paths: &[PathBuf], cancel: &AtomicBool) -> (f64, Vec<f64>) {
    let mut per_file = Vec::with_capacity(paths.len());
    let t = Instant::now();
    for p in paths {
        let t0 = Instant::now();
        Blake3Hasher.hash_file(p, cancel).expect("hash");
        per_file.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    (t.elapsed().as_secs_f64(), per_file)
}

/// 캐시를 우회하는 순수 읽기. 버퍼는 4 KiB 정렬, 읽기 크기는 섹터 배수.
fn uncached_read(paths: &[PathBuf]) -> f64 {
    let mut raw = vec![0u8; 8 * MIB as usize + 4096];
    let off = raw.as_ptr().align_offset(4096);
    let buf = &mut raw[off..off + 8 * MIB as usize];
    let t = Instant::now();
    for p in paths {
        let mut f = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING)
            .open(p)
            .expect("open uncached");
        while f.read(buf).expect("read uncached") > 0 {}
    }
    t.elapsed().as_secs_f64()
}

fn cpu_ceiling_mb_s() -> f64 {
    let buf = vec![0xA5u8; 256 * MIB as usize];
    let t = Instant::now();
    let mut h = blake3::Hasher::new();
    for _ in 0..4 {
        h.update(&buf);
    }
    let digest = h.finalize();
    let secs = t.elapsed().as_secs_f64();
    std::hint::black_box(digest);
    1024.0 / secs // 1 GiB = 1024 MiB
}

fn ps(script: &str) -> String {
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn conditions(dir: &Path) -> String {
    let drive = fs::canonicalize(dir)
        .ok()
        .and_then(|p| p.to_string_lossy().chars().nth(4))
        .unwrap_or('C');
    ps(&format!(
        "$c=Get-CimInstance Win32_Processor|Select-Object -First 1; \
         $r=[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1); \
         $p=Get-Partition -DriveLetter {drive}|Get-Disk|Get-PhysicalDisk; \
         $v=Get-Volume -DriveLetter {drive}; \
         \"CPU: $($c.Name.Trim()) ($($c.NumberOfCores)C/$($c.NumberOfLogicalProcessors)T)`nRAM: $r GB`n\
         Disk: $($p.FriendlyName) [$($p.MediaType)/$($p.BusType)] fs=$($v.FileSystem) free=$([math]::Round($v.SizeRemaining/1GB,1)) GB\""
    ))
}

fn main() {
    let mut dir = PathBuf::from("target/bench-p1-4");
    let mut huge_gib: u64 = 8;
    let mut keep = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = PathBuf::from(args.next().expect("--dir value")),
            "--huge-gib" => huge_gib = args.next().expect("value").parse().expect("number"),
            "--keep" => keep = true,
            other => panic!("unknown arg {other}"),
        }
    }
    fs::create_dir_all(&dir).unwrap();
    println!("== P1-4 hash I/O ==\n{}", conditions(&dir));
    println!("build: release={}", !cfg!(debug_assertions));
    println!(
        "in-memory BLAKE3 single-thread ceiling: {:.0} MiB/s\n",
        cpu_ceiling_mb_s()
    );

    let mut classes = vec![
        Class {
            name: "small",
            size: 128 * 1024,
            count: 4000,
        },
        Class {
            name: "medium",
            size: 10 * MIB,
            count: 100,
        },
        Class {
            name: "large",
            size: 200 * MIB,
            count: 10,
        },
    ];
    if huge_gib > 0 {
        classes.push(Class {
            name: "huge",
            size: huge_gib * 1024 * MIB,
            count: 1,
        });
    }
    let cancel = AtomicBool::new(false);

    println!(
        "| class | files x size | total MiB | gen s | pass1 MiB/s | pass2 MiB/s | uncached read MiB/s | per-file ms P50/P95/P99 (pass2) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for c in &classes {
        let sub = dir.join(c.name);
        let t = Instant::now();
        let paths = generate(&sub, c).expect("generate");
        let gen_s = t.elapsed().as_secs_f64();
        let total_mib = (c.size * c.count as u64) as f64 / MIB as f64;

        let (s1, _) = hash_pass(&paths, &cancel);
        let (s2, mut per) = hash_pass(&paths, &cancel);
        let sr = uncached_read(&paths);
        per.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!(
            "| {} | {} x {:.2} MiB | {:.0} | {:.1} | {:.0} | {:.0} | {:.0} | {:.2} / {:.2} / {:.2} |",
            c.name,
            c.count,
            c.size as f64 / MIB as f64,
            total_mib,
            gen_s,
            total_mib / s1,
            total_mib / s2,
            total_mib / sr,
            percentile(&per, 0.50),
            percentile(&per, 0.95),
            percentile(&per, 0.99),
        );
        if !keep {
            fs::remove_dir_all(&sub).unwrap();
        }
    }
    if !keep {
        let _ = fs::remove_dir(&dir);
    }
}
