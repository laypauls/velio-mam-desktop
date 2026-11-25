//! 보고 출력(마크다운 표)과 시나리오 공용 도우미.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::metrics::{Conditions, Snap, conditions, logical_cpus, proc_now};
use super::queries::{KINDS, Report};
use super::stats::{linreg, mean, summarize};

/// 측정 데이터 루트. `MAM_P4_DIR` 로 바꿀 수 있다 (기본 `D:\mam-bench\p4-load`). 사용자 폴더는 쓰지 않는다.
pub fn bench_root() -> PathBuf {
    std::env::var_os("MAM_P4_DIR")
        .map_or_else(|| PathBuf::from(r"D:\mam-bench\p4-load"), PathBuf::from)
}

pub fn env_num(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.replace('_', "").parse().ok())
        .unwrap_or(default)
}

/// `cond` 가 참이 될 때까지 폴링한다.
pub fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    cond()
}

/// 폴더 크기 (바이트, 재귀).
pub fn dir_bytes(root: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

pub fn gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// 다른 빌드 프로세스 수와 짧은 구간의 시스템 CPU — 측정 시점의 기계 부하 기록.
pub fn machine_load() -> String {
    let a = proc_now();
    std::thread::sleep(Duration::from_secs(3));
    let b = proc_now();
    let d_total = b.sys_total.saturating_sub(a.sys_total) as f64;
    let d_idle = b.sys_idle.saturating_sub(a.sys_idle) as f64;
    let cpu = if d_total > 0.0 {
        100.0 * (1.0 - d_idle / d_total)
    } else {
        0.0
    };
    let builds = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Process cargo,rustc,link,cl,rust-lld -ErrorAction SilentlyContinue | Measure-Object).Count",
        ])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    format!("시스템 CPU {cpu:.0}% (3초), cargo/rustc/link 계열 프로세스 {builds}개")
}

pub fn print_conditions(title: &str, extra: &[(&str, String)]) {
    let c: Conditions = conditions();
    println!("\n## {title}");
    println!("- CPU: {} / 논리 CPU {}", c.cpu, logical_cpus());
    println!("- RAM: {} GB / OS: {}", c.ram_gb, c.os);
    println!("- 디스크: {} / 전원: {}", c.disks, c.power_plan);
    println!("- 기계 부하: {}", machine_load());
    for (k, v) in extra {
        println!("- {k}: {v}");
    }
}

/// 종류별 P50/P95/P99 표.
pub fn print_query_table(label: &str, rep: &Report) {
    println!("\n### {label}");
    println!(
        "표본 {} 건, 오류 {}, 하이브리드 degraded {}/{}, 벽시계 {:.0}s",
        rep.total(),
        rep.errors,
        rep.hybrid_degraded,
        rep.hybrid_total,
        rep.wall_s
    );
    if !rep.first_errors.is_empty() {
        println!("첫 오류: {:?}", rep.first_errors);
    }
    println!("| 질의 | n | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|");
    for (k, name) in KINDS.iter().enumerate() {
        let s = rep.summary(k);
        println!(
            "| {name} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
            s.n, s.p50, s.p95, s.p99, s.max
        );
    }
    let all = rep.all();
    println!(
        "| **전체** | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
        all.n, all.p50, all.p95, all.p99, all.max
    );
    // 하이브리드를 뺀 키워드 계열
    let kw: Vec<f64> = rep
        .samples
        .iter()
        .enumerate()
        .filter(|(k, _)| *k != super::queries::HYBRID)
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    let s = summarize(&kw);
    println!(
        "| 키워드 계열 합계 (하이브리드 제외) | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
        s.n, s.p50, s.p95, s.p99, s.max
    );
}

/// 표본 시계열 요약 (30초 간격으로 줄여 출력).
pub fn print_timeline(snaps: &[Snap], every: usize) {
    println!(
        "| t(s) | RSS MB | 커밋 MB | 핸들 | 스레드 | 프로세스 CPU(코어) | 시스템 CPU % | outbox pending | 작업 pending | 작업 running |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (i, s) in snaps.iter().enumerate() {
        if i % every.max(1) == 0 || i + 1 == snaps.len() {
            println!(
                "| {:.0} | {:.0} | {:.0} | {} | {} | {:.2} | {:.0} | {} | {} | {} |",
                s.t_s,
                s.rss_mb,
                s.commit_mb,
                s.handles,
                s.threads,
                s.proc_cores,
                s.sys_cpu_pct,
                s.outbox_pending,
                s.jobs_pending,
                s.jobs_running
            );
        }
    }
    if let Some(peak) = snaps.iter().map(|s| s.peak_mb).reduce(f64::max) {
        let cores: Vec<f64> = snaps.iter().map(|s| s.proc_cores).collect();
        let sys: Vec<f64> = snaps.iter().map(|s| s.sys_cpu_pct).collect();
        let mx = snaps.iter().map(|s| s.outbox_pending).max().unwrap_or(0);
        println!(
            "\n요약: 최대 작업 집합 {peak:.0} MB, 프로세스 CPU 평균 {:.2} 코어(최대 {:.2}), 시스템 CPU 평균 {:.0}%, outbox pending 최대 {mx}, 스레드 최대 {}, 핸들 최대 {}",
            mean(&cores),
            cores.iter().copied().fold(0.0, f64::max),
            mean(&sys),
            snaps.iter().map(|s| s.threads).max().unwrap_or(0),
            snaps.iter().map(|s| s.handles).max().unwrap_or(0),
        );
    }
}

/// 추세: 값열의 선형 회귀 기울기(단위/표본)와 R², 앞 25%·뒤 25% 평균.
pub fn trend_line(name: &str, ys: &[f64], per_round_to_hour: f64) -> String {
    let (slope, r2) = linreg(ys);
    let q = (ys.len() / 4).max(1);
    let first = mean(&ys[..q.min(ys.len())]);
    let last = mean(&ys[ys.len().saturating_sub(q)..]);
    format!(
        "| {name} | {first:.1} | {last:.1} | {:.4} | {:.3} | {:.2} |",
        slope,
        r2,
        slope * per_round_to_hour
    )
}
