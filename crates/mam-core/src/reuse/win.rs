//! Windows 구현 — 기본 프로그램으로 열기(ShellExecute), 탐색기 선택, CF_HDROP 클립보드.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use super::{Clipboard, Launcher, clipboard::build_drop_files};

/// 표준 클립보드 형식 CF_HDROP
const CF_HDROP: u32 = 15;
/// 다른 프로그램이 클립보드를 잡고 있을 때 재시도
const OPEN_TRIES: u32 = 10;
const OPEN_WAIT: Duration = Duration::from_millis(25);

pub struct ShellLauncher;

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

impl Launcher for ShellLauncher {
    fn open(&self, path: &Path) -> io::Result<()> {
        let target = wide(path);
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // ShellExecute 는 호출 스레드의 COM 초기화를 기대한다 — 앱 스레드 상태를 건드리지 않도록 전용 스레드에서 호출한다
        let result = std::thread::spawn(move || {
            // SAFETY: 모든 포인터는 NUL 종료된 UTF-16 버퍼(호출 동안 유효)이거나 null 이다.
            unsafe {
                let init = CoInitializeEx(
                    std::ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
                );
                let code = ShellExecuteW(
                    std::ptr::null_mut(),
                    verb.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                ) as usize;
                if init >= 0 {
                    CoUninitialize();
                }
                code
            }
        })
        .join()
        .map_err(|_| io::Error::other("shell launch thread panicked"))?;
        // 32 이하는 오류 코드 (ShellExecute 규약)
        if result > 32 {
            Ok(())
        } else {
            Err(io::Error::other(format!("ShellExecute failed ({result})")))
        }
    }

    fn reveal(&self, path: &Path) -> io::Result<()> {
        // `explorer.exe /select,"<경로>"` — 인자 인용을 직접 지정한다 (exit code 는 성공해도 1 이라 확인하지 않는다)
        let arg = format!("/select,\"{}\"", path.display());
        std::process::Command::new("explorer.exe")
            .raw_arg(arg)
            .spawn()
            .map(|_| ())
    }
}

/// 메인 창 핸들을 소유자로 클립보드를 연다 (소유자가 null 이면 `SetClipboardData` 가 실패한다).
pub struct WindowsClipboard {
    pub hwnd: isize,
}

impl Clipboard for WindowsClipboard {
    fn set_files(&self, paths: &[PathBuf]) -> io::Result<()> {
        let bytes = build_drop_files(paths)?;
        // SAFETY: 클립보드를 연 동안에만 접근하고, 성공한 SetClipboardData 이후 메모리 소유권은 시스템에 넘어간다.
        unsafe {
            let mut opened = false;
            for _ in 0..OPEN_TRIES {
                if OpenClipboard(self.hwnd as _) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(OPEN_WAIT);
            }
            if !opened {
                return Err(io::Error::last_os_error());
            }
            let result = (|| {
                if EmptyClipboard() == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
                if mem.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let dst = GlobalLock(mem).cast::<u8>();
                if dst.is_null() {
                    let err = io::Error::last_os_error();
                    GlobalFree(mem);
                    return Err(err);
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len());
                GlobalUnlock(mem);
                if SetClipboardData(CF_HDROP, mem).is_null() {
                    let err = io::Error::last_os_error();
                    GlobalFree(mem);
                    return Err(err);
                }
                Ok(())
            })();
            CloseClipboard();
            result
        }
    }
}
