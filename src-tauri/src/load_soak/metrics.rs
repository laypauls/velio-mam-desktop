//! 실행 조건과 프로세스 지표 — RSS·커밋·핸들·스레드·CPU (Win32). `crates/*/examples/common/sysinfo.rs` 를 바탕으로 했다.
//!
//! windows-sys 의 필요한 feature 가 `src-tauri` 에 켜져 있지 않아(제품 의존성은 바꾸지 않는다) kernel32 함수를 직접 선언한다.
//! 사이드카(ffmpeg/ffprobe) 자식 프로세스의 CPU·메모리는 이 프로세스 지표에 포함되지 않는다 — 시스템 CPU 로만 보인다.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Conditions {
    pub cpu: String,
    pub ram_gb: String,
    pub os: String,
    pub power_plan: String,
    pub disks: String,
}

fn powershell(script: &str) -> String {
    let full = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &full])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn power_plan() -> String {
    let out = Command::new("powercfg")
        .arg("/getactivescheme")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let guid = out
        .split_whitespace()
        .find(|t| t.len() == 36 && t.matches('-').count() == 4)
        .unwrap_or("")
        .to_lowercase();
    let name = match guid.as_str() {
        "381b4222-f694-41f0-9685-ff5bb260df2e" => "Balanced",
        "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c" => "High performance",
        "a1841308-3541-4fab-bc81-f71556f20b4a" => "Power saver",
        "e9a42b02-d5df-448d-aa00-03f14749eb61" => "Ultimate performance",
        "" => "unknown",
        _ => "custom",
    };
    format!("{name} ({guid})")
}

pub fn conditions() -> Conditions {
    let info = powershell(
        "$c=Get-CimInstance Win32_Processor|Select-Object -First 1; \
         $r=[math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory/1GB,1); \
         $o=Get-CimInstance Win32_OperatingSystem; \
         $d=(Get-PhysicalDisk|ForEach-Object{\"$($_.FriendlyName) [$($_.MediaType)]\"}) -join '; '; \
         \"$($c.Name.Trim()) ($($c.NumberOfCores)C/$($c.NumberOfLogicalProcessors)T)|$r|$($o.Caption) $($o.Version)|$d\"",
    );
    let mut it = info.split('|').map(str::to_string);
    let mut next = || it.next().unwrap_or_else(|| "unknown".to_string());
    Conditions {
        cpu: next(),
        ram_gb: next(),
        os: next(),
        disks: next(),
        power_plan: power_plan(),
    }
}

/// 논리 CPU 수 (CPU 사용률을 코어 환산할 때 쓴다)
pub fn logical_cpus() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// 한 시점의 프로세스 지표.
#[derive(Clone, Copy, Debug, Default)]
pub struct Proc {
    /// 작업 집합(RSS) 바이트
    pub rss: u64,
    pub peak_rss: u64,
    /// 커밋(pagefile usage) 바이트
    pub commit: u64,
    pub handles: u32,
    pub threads: u32,
    /// 사용자+커널 CPU 시간 (100ns)
    pub cpu_100ns: u64,
    /// 시스템 전체 (idle, busy=kernel+user-idle... 누적 100ns)
    pub sys_idle: u64,
    pub sys_total: u64,
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(windows)]
mod ffi {
    #[repr(C)]
    pub struct MemCounters {
        pub cb: u32,
        pub page_fault_count: u32,
        pub peak_working_set: usize,
        pub working_set: usize,
        pub quota_peak_paged: usize,
        pub quota_paged: usize,
        pub quota_peak_non_paged: usize,
        pub quota_non_paged: usize,
        pub pagefile: usize,
        pub peak_pagefile: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct FileTime {
        pub lo: u32,
        pub hi: u32,
    }

    impl FileTime {
        pub fn as_u64(self) -> u64 {
            (u64::from(self.hi) << 32) | u64::from(self.lo)
        }
    }

    #[repr(C)]
    pub struct ThreadEntry {
        pub size: u32,
        pub usage: u32,
        pub thread_id: u32,
        pub owner_pid: u32,
        pub base_pri: i32,
        pub delta_pri: i32,
        pub flags: u32,
    }

    pub const TH32CS_SNAPTHREAD: u32 = 0x4;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetCurrentProcess() -> isize;
        pub fn GetCurrentProcessId() -> u32;
        pub fn K32GetProcessMemoryInfo(h: isize, c: *mut MemCounters, cb: u32) -> i32;
        pub fn GetProcessHandleCount(h: isize, count: *mut u32) -> i32;
        pub fn GetProcessTimes(
            h: isize,
            create: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        pub fn GetSystemTimes(
            idle: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        pub fn Thread32First(h: isize, e: *mut ThreadEntry) -> i32;
        pub fn Thread32Next(h: isize, e: *mut ThreadEntry) -> i32;
        pub fn CloseHandle(h: isize) -> i32;
    }
}

#[cfg(windows)]
fn thread_count(pid: u32) -> u32 {
    use ffi::*;
    // SAFETY: 스냅샷 핸들은 이 함수 안에서만 쓰고 닫는다. 구조체는 크기를 채워 초기화한다.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == -1 {
            return 0;
        }
        let mut e = ThreadEntry {
            size: std::mem::size_of::<ThreadEntry>() as u32,
            usage: 0,
            thread_id: 0,
            owner_pid: 0,
            base_pri: 0,
            delta_pri: 0,
            flags: 0,
        };
        let mut n = 0;
        let mut ok = Thread32First(snap, &mut e);
        while ok != 0 {
            if e.owner_pid == pid {
                n += 1;
            }
            e.size = std::mem::size_of::<ThreadEntry>() as u32;
            ok = Thread32Next(snap, &mut e);
        }
        CloseHandle(snap);
        n
    }
}

#[cfg(windows)]
pub fn proc_now() -> Proc {
    use ffi::*;
    // SAFETY: 현재 프로세스 의사 핸들과 0 초기화한 출력 구조체만 사용한다.
    unsafe {
        let h = GetCurrentProcess();
        let mut m: MemCounters = std::mem::zeroed();
        m.cb = std::mem::size_of::<MemCounters>() as u32;
        let have_mem = K32GetProcessMemoryInfo(h, &mut m, m.cb) != 0;
        let mut handles = 0u32;
        GetProcessHandleCount(h, &mut handles);
        let (mut c, mut e, mut k, mut u) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        let (mut idle, mut sk, mut su) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        GetSystemTimes(&mut idle, &mut sk, &mut su);
        Proc {
            rss: if have_mem { m.working_set as u64 } else { 0 },
            peak_rss: if have_mem {
                m.peak_working_set as u64
            } else {
                0
            },
            commit: if have_mem { m.pagefile as u64 } else { 0 },
            handles,
            threads: thread_count(GetCurrentProcessId()),
            cpu_100ns: k.as_u64() + u.as_u64(),
            sys_idle: idle.as_u64(),
            // GetSystemTimes 의 kernel 시간에는 idle 이 포함된다
            sys_total: sk.as_u64() + su.as_u64(),
        }
    }
}

#[cfg(not(windows))]
pub fn proc_now() -> Proc {
    Proc::default()
}

/// 주기 표본 1건.
#[derive(Clone, Copy, Debug, Default)]
pub struct Snap {
    pub t_s: f64,
    pub rss_mb: f64,
    pub peak_mb: f64,
    pub commit_mb: f64,
    pub handles: u32,
    pub threads: u32,
    /// 이 프로세스가 쓴 CPU (코어 환산: 1.0 = 한 코어 100%)
    pub proc_cores: f64,
    /// 시스템 전체 CPU 사용률 % (다른 프로세스·세션 포함)
    pub sys_cpu_pct: f64,
    pub outbox_pending: i64,
    pub jobs_pending: u64,
    pub jobs_running: u64,
}

/// 표본마다 호출되는 탐침: (outbox pending, 작업 pending, 작업 running).
pub type Probe = dyn Fn() -> (i64, u64, u64) + Send + Sync;

/// 백그라운드 표본 수집기. `finish` 가 스레드를 멈추고 표본을 돌려준다.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Vec<Snap>>,
}

impl Sampler {
    pub fn start(interval: Duration, probe: Arc<Probe>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("p4-sampler".into())
            .spawn(move || {
                let t0 = Instant::now();
                let mut out = Vec::new();
                let mut prev = proc_now();
                let mut prev_at = Instant::now();
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(interval);
                    let now = proc_now();
                    let wall = prev_at.elapsed().as_secs_f64().max(1e-6);
                    let cpu_s = now.cpu_100ns.saturating_sub(prev.cpu_100ns) as f64 / 1e7;
                    let d_total = now.sys_total.saturating_sub(prev.sys_total) as f64;
                    let d_idle = now.sys_idle.saturating_sub(prev.sys_idle) as f64;
                    let (outbox, pend, run) = probe();
                    out.push(Snap {
                        t_s: t0.elapsed().as_secs_f64(),
                        rss_mb: mb(now.rss),
                        peak_mb: mb(now.peak_rss),
                        commit_mb: mb(now.commit),
                        handles: now.handles,
                        threads: now.threads,
                        proc_cores: cpu_s / wall,
                        sys_cpu_pct: if d_total > 0.0 {
                            100.0 * (1.0 - d_idle / d_total)
                        } else {
                            0.0
                        },
                        outbox_pending: outbox,
                        jobs_pending: pend,
                        jobs_running: run,
                    });
                    prev = now;
                    prev_at = Instant::now();
                }
                out
            })
            .expect("spawn sampler");
        Self { stop, thread }
    }

    pub fn finish(self) -> Vec<Snap> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.join().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (Windows 지표가 0 이 아니다)
    use super::*;

    #[cfg(windows)]
    #[test]
    fn t_p4_metrics_01_process_counters_are_populated() {
        // Arrange / Act
        let p = proc_now();
        // Assert
        assert!(p.rss > 0 && p.handles > 0 && p.threads > 0 && p.cpu_100ns > 0);
        assert!(p.peak_rss >= p.rss);
    }
}
