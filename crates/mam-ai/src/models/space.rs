//! 저장 폴더의 여유 공간 조회 — trait 으로 분리해 디스크 부족을 시험에서 모사한다.

use std::path::Path;

pub trait SpaceProbe: Send + Sync {
    /// `dir`(없으면 가장 가까운 상위 폴더)가 있는 볼륨의 사용 가능 바이트. 알 수 없으면 `None`.
    fn available(&self, dir: &Path) -> Option<u64>;
}

/// OS 조회 (Windows: `GetDiskFreeSpaceExW`). 다른 플랫폼은 `None` (검사 생략).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSpace;

impl SpaceProbe for SystemSpace {
    #[cfg(windows)]
    fn available(&self, dir: &Path) -> Option<u64> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let existing = dir.ancestors().find(|p| p.exists())?;
        let wide: Vec<u16> = existing.as_os_str().encode_wide().chain([0]).collect();
        let mut free_to_caller = 0u64;
        // SAFETY: wide 는 NUL 로 끝나는 유효한 경로이고, 출력 포인터는 이 스택 변수 하나뿐이다 (나머지는 NULL 허용).
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_to_caller,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(free_to_caller)
    }

    #[cfg(not(windows))]
    fn available(&self, _dir: &Path) -> Option<u64> {
        None
    }
}
