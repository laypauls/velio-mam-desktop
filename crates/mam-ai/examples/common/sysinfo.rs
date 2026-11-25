//! 실행 조건(CPU·RAM·OS·전원 모드·디스크)과 RSS — mam-search `examples/common/sysinfo.rs` 복사 + 현재 작업 집합 추가.

use std::process::Command;

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

/// `powercfg /getactivescheme` 의 GUID 를 알려진 이름으로 바꾼다 (출력 로케일 의존 제거).
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

/// 프로세스 최대 작업 집합(바이트). 지원하지 않는 플랫폼은 0.
#[cfg(windows)]
pub fn peak_rss_bytes() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: 구조체를 0 초기화하고 cb 에 크기를 넣은 뒤 현재 프로세스 의사 핸들만 사용한다.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            c.PeakWorkingSetSize as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
pub fn peak_rss_bytes() -> u64 {
    0
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// 프로세스 현재 작업 집합(바이트) — view(메모리 매핑)로 연 직후와 질의 후를 비교하는 데 쓴다.
#[cfg(windows)]
pub fn current_rss_bytes() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: peak_rss_bytes 와 같다.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) != 0 {
            c.WorkingSetSize as u64
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
pub fn current_rss_bytes() -> u64 {
    0
}
