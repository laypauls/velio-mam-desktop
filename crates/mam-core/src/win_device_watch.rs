//! Windows 장치 변경 감지 — 전용 스레드의 숨긴 **최상위** 창이 `WM_DEVICECHANGE` 를 받는다.
//!
//! `DBT_DEVICEARRIVAL` / `DBT_DEVICEREMOVECOMPLETE` (`DBT_DEVTYP_VOLUME`) 는 최상위 창에만 브로드캐스트된다 —
//! message-only 창(`HWND_MESSAGE`)은 받지 못한다. 창은 만들기만 하고 표시하지 않는다(Tauri 메인 창과 무관하게
//! 수명이 독립). 이벤트는 볼륨 정보를 읽지 않고 콜백만 부른다 ([`crate::device_watch`]).

use std::any::Any;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::mpsc::channel;
use std::thread::JoinHandle;
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME,
    DEV_BROADCAST_HDR, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage,
    WM_CLOSE, WM_DESTROY, WM_DEVICECHANGE, WNDCLASSW, WS_OVERLAPPED,
};

use crate::device_watch::{DeviceNotify, DeviceWatcher};
use crate::error::CoreError;

static REGISTER: std::sync::Once = std::sync::Once::new();
const CLASS_NAME: &str = "MamDeviceWatcherWindow";
/// 시작 응답 대기 상한 (창 생성은 즉시 끝난다 — 넘으면 실패로 보고 주기 점검만 쓴다)
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDeviceWatcher;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

thread_local! {
    /// 창 프로시저(같은 스레드에서 실행)가 부를 콜백
    static NOTIFY: std::cell::RefCell<Option<DeviceNotify>> = const { std::cell::RefCell::new(None) };
}

/// 볼륨 연결·분리 메시지인가 (다른 장치 종류·기타 `WM_DEVICECHANGE` 는 무시).
fn is_volume_change(wparam: WPARAM, lparam: LPARAM) -> bool {
    if wparam as u32 != DBT_DEVICEARRIVAL && wparam as u32 != DBT_DEVICEREMOVECOMPLETE {
        return false;
    }
    if lparam == 0 {
        return false;
    }
    // SAFETY: OS 가 DBT_DEVICEARRIVAL/REMOVECOMPLETE 에서 유효한 DEV_BROADCAST_HDR 포인터를 준다
    // (시험의 SendMessage 도 같은 형태를 넘긴다). 헤더 크기만큼만 읽는다.
    let hdr = unsafe { &*(lparam as *const DEV_BROADCAST_HDR) };
    (hdr.dbch_size as usize) >= std::mem::size_of::<DEV_BROADCAST_HDR>()
        && hdr.dbch_devicetype == DBT_DEVTYP_VOLUME
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_DEVICECHANGE => {
            if is_volume_change(wparam, lparam) {
                let cb = NOTIFY.with(|n| n.borrow().clone());
                if let Some(cb) = cb {
                    cb();
                }
            }
            1 // TRUE: 계속 진행 허용
        }
        WM_CLOSE => {
            // SAFETY: 이 스레드가 만든 창이다.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            // SAFETY: 메시지 루프를 끝낸다.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: 기본 처리.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 실행 중인 감지 스레드. drop 하면 창을 닫고 스레드를 기다린다.
pub struct DeviceWatchHandle {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
}

impl DeviceWatchHandle {
    #[cfg(test)]
    fn hwnd(&self) -> HWND {
        self.hwnd as HWND
    }
}

impl Drop for DeviceWatchHandle {
    fn drop(&mut self) {
        // SAFETY: 다른 스레드에서도 PostMessage 는 안전하다. 창이 이미 없으면 실패할 뿐이다.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run_window(notify: DeviceNotify, ready: std::sync::mpsc::Sender<Result<isize, CoreError>>) {
    let class = wide(CLASS_NAME);
    // SAFETY: NULL 은 현재 실행 파일의 모듈 핸들.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
    };
    // 클래스는 프로세스 수명 동안 한 번만 등록한다 (병렬 시작·해제 사이의 해제 경합을 피한다)
    REGISTER.call_once(|| {
        // SAFETY: wc 의 포인터들은 이 호출 동안 유효하다. 이미 등록돼 실패해도 창 생성은 그대로 된다.
        unsafe { RegisterClassW(&wc) };
    });
    // SAFETY: 부모·메뉴 없는 일반 최상위 창. 표시(ShowWindow)하지 않으므로 보이지 않는다.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        let _ = ready.send(Err(CoreError::Volume(
            "cannot create the device watcher window".into(),
        )));
        return;
    }
    NOTIFY.with(|n| *n.borrow_mut() = Some(notify));
    let _ = ready.send(Ok(hwnd as isize));
    // SAFETY: 이 스레드의 메시지 큐를 WM_QUIT 까지 돌린다.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    NOTIFY.with(|n| *n.borrow_mut() = None);
}

impl DeviceWatcher for WindowsDeviceWatcher {
    fn start(&self, notify: DeviceNotify) -> Result<Box<dyn Any + Send>, CoreError> {
        let (tx, rx) = channel();
        let thread = std::thread::Builder::new()
            .name("mam-device-watch".into())
            .spawn(move || run_window(notify, tx))
            .map_err(|e| CoreError::Volume(format!("cannot start the device watcher: {e}")))?;
        let hwnd = match rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                return Err(CoreError::Volume(
                    "device watcher did not start in time".into(),
                ));
            }
        };
        Ok(Box::new(DeviceWatchHandle {
            hwnd,
            thread: Some(thread),
        }))
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (알림 전달·정상 종료) / Validation (볼륨 외 장치·다른 wparam 무시) / External (재시작)
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

    use super::*;

    fn start(counter: &Arc<AtomicUsize>) -> Box<dyn Any + Send> {
        let c = Arc::clone(counter);
        WindowsDeviceWatcher
            .start(Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }))
            .unwrap()
    }

    fn send(h: &dyn Any, wparam: u32, devtype: u32, size: u32) {
        let h = h.downcast_ref::<DeviceWatchHandle>().unwrap();
        let hdr = DEV_BROADCAST_HDR {
            dbch_size: size,
            dbch_devicetype: devtype,
            dbch_reserved: 0,
        };
        // SAFETY: SendMessage 는 창 프로시저가 끝날 때까지 블록하므로 hdr 는 그 동안 살아 있다.
        unsafe {
            SendMessageW(
                h.hwnd(),
                WM_DEVICECHANGE,
                wparam as WPARAM,
                &hdr as *const _ as LPARAM,
            );
        }
    }

    const HDR: u32 = std::mem::size_of::<DEV_BROADCAST_HDR>() as u32;

    #[test]
    fn t_devwatch_win_01_volume_arrival_and_removal_notify() {
        // Arrange
        let n = Arc::new(AtomicUsize::new(0));
        let h = start(&n);

        // Act
        send(&*h, DBT_DEVICEARRIVAL, DBT_DEVTYP_VOLUME, HDR);
        send(&*h, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME, HDR);

        // Assert (SendMessage 는 동기라 폴링 없이 확인 가능)
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn t_devwatch_win_02_non_volume_devices_and_other_events_are_ignored() {
        // Arrange
        let n = Arc::new(AtomicUsize::new(0));
        let h = start(&n);

        // Act: 포트(DBT_DEVTYP_PORT=3), 다른 wparam(QUERYREMOVE=0x8001), 잘린 헤더
        send(&*h, DBT_DEVICEARRIVAL, 3, HDR);
        send(&*h, 0x8001, DBT_DEVTYP_VOLUME, HDR);
        send(&*h, DBT_DEVICEARRIVAL, DBT_DEVTYP_VOLUME, 4);

        // Assert
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn t_devwatch_win_03_drop_stops_thread_and_restart_works() {
        // Arrange / Act: 시작 -> 종료 -> 다시 시작을 반복해도 매번 정상 종료된다 (join 이 끝나면 통과)
        for _ in 0..3 {
            let n = Arc::new(AtomicUsize::new(0));
            let h = start(&n);
            send(&*h, DBT_DEVICEARRIVAL, DBT_DEVTYP_VOLUME, HDR);
            assert_eq!(n.load(Ordering::SeqCst), 1);
            drop(h);
        }
    }
}
