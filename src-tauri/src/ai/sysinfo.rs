//! 시스템 정보 두 가지 — 물리 RAM(자원 기본값 선택, P1-6)과 로컬 UTC 오프셋(예약 시간대 판정).

/// 물리 RAM(GB). 알 수 없으면 0 (저사양으로 취급된다 — `ResourceConfig::for_ram_gb`).
pub fn total_ram_gb() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: 구조체를 0 으로 채우고 dwLength 를 설정한 뒤 OS 가 채우게 한다.
        let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut s) } != 0 {
            return s.ullTotalPhys / (1024 * 1024 * 1024);
        }
    }
    0
}

/// 현재 로컬 시각 - UTC (분). 알 수 없으면 0.
pub fn local_utc_offset_min() -> i32 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        // SAFETY: 0 으로 채운 구조체를 OS 가 채운다.
        let mut tz: TIME_ZONE_INFORMATION = unsafe { std::mem::zeroed() };
        let id = unsafe { GetTimeZoneInformation(&mut tz) };
        // 1 = 표준시, 2 = 일광 절약 시간 (0 = 알 수 없음, u32::MAX = 실패)
        let extra = match id {
            1 => tz.StandardBias,
            2 => tz.DaylightBias,
            0 => 0,
            _ => return 0,
        };
        -(tz.Bias + extra)
    }
    #[cfg(not(windows))]
    0
}
