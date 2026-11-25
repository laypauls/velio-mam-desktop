//! Windows 세션 종료(종료·재시작·로그오프) 처리.
//!
//! 트레이에 상주하는 앱은 PC 를 끌 때 `RunEvent::Exit` 없이 프로세스가 끝나기 쉽다. 종료 정리가 돌지 못하면 시작 표식
//! (`ingest_run_marker`)이 남아 다음 부팅이 실시간 감시 폴더를 전부 재스캔한다.
//!
//! - 메시지 판정(`decide`)은 Win32 없이 시험할 수 있는 순수 함수다.
//! - 메인 창을 서브클래싱해 `WM_QUERYENDSESSION`(허용 응답)·`WM_ENDSESSION`(wParam=TRUE → 짧은 기한 정리)을 받는다.
//! - tao 0.37 은 숨김 이벤트 창에서 `WM_ENDSESSION(TRUE)` 를 받아 `RunEvent::Exit` 를 일으킨 뒤 `process::exit(0)` 한다.
//!   어느 창이 먼저 받는지는 정해져 있지 않으므로, `RunEvent::Exit` 쪽도 `is_ending()` 으로 세션 종료를 알아본다.

pub const WM_QUERYENDSESSION: u32 = 0x0011;
pub const WM_ENDSESSION: u32 = 0x0016;

/// 세션 종료 메시지에 대한 처리 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 종료를 허용한다고 응답한다 (`WM_QUERYENDSESSION` → TRUE). 앱은 종료를 막지 않는다.
    Allow,
    /// 세션이 실제로 끝난다 — 짧은 기한으로 정리를 실행한다.
    Shutdown,
    /// 다른 메시지이거나 종료가 취소됐다 (`WM_ENDSESSION` wParam=FALSE) — 아무것도 하지 않는다.
    Ignore,
}

pub fn decide(msg: u32, wparam: usize) -> Decision {
    match msg {
        WM_QUERYENDSESSION => Decision::Allow,
        WM_ENDSESSION if wparam != 0 => Decision::Shutdown,
        _ => Decision::Ignore,
    }
}

#[cfg(windows)]
pub use win::{install, is_ending};

#[cfg(not(windows))]
pub fn is_ending() -> bool {
    false
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_SHUTTINGDOWN, WM_NCDESTROY,
    };

    use super::{Decision, decide};

    const SUBCLASS_ID: usize = 0x4d41_4d53; // "MAMS"

    static ENDING: AtomicBool = AtomicBool::new(false);
    static ON_SESSION_END: OnceLock<Arc<dyn Fn() + Send + Sync>> = OnceLock::new();

    /// 세션이 끝나는 중인가 — 서브클래스가 `WM_ENDSESSION(TRUE)` 를 봤거나 OS 가 세션 종료 중이라고 답한다.
    pub fn is_ending() -> bool {
        // SAFETY: 인자 없는 조회 함수
        ENDING.load(Ordering::SeqCst) || unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) } != 0
    }

    /// 메인 창에 세션 종료 처리기를 건다. UI 스레드(창을 만든 스레드)에서 호출한다.
    /// `on_end` 는 `WM_ENDSESSION(TRUE)` 에서 UI 스레드가 동기 실행한다 — 짧은 기한 정리여야 한다.
    pub fn install(hwnd: *mut c_void, on_end: Arc<dyn Fn() + Send + Sync>) -> bool {
        if ON_SESSION_END.set(on_end).is_err() {
            return false;
        }
        // SAFETY: hwnd 는 호출 스레드가 소유한 유효한 창이고, 콜백은 정적 함수다
        unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0) != 0 }
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match decide(msg, wparam) {
            Decision::Allow => return 1,
            Decision::Shutdown => {
                ENDING.store(true, Ordering::SeqCst);
                if let Some(f) = ON_SESSION_END.get() {
                    // 패닉이 창 프로시저 밖(FFI 경계)으로 새지 않게 한다
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f()));
                }
                // 처리했음(0) — 원래 프로시저(tao)에도 전달한다 (tao 는 이어서 Exit 를 일으키지만 정리는 한 번만 돈다)
            }
            Decision::Ignore => {
                if msg == WM_NCDESTROY {
                    // SAFETY: 같은 창·같은 ID 로 건 서브클래스를 푼다
                    unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID) };
                }
            }
        }
        // SAFETY: 서브클래스 체인의 다음 처리기로 그대로 넘긴다
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary / Business rule
    use super::*;

    #[test]
    fn t_sessend_01_query_end_session_is_allowed_regardless_of_params() {
        // Arrange / Act / Assert
        assert_eq!(decide(WM_QUERYENDSESSION, 0), Decision::Allow);
        assert_eq!(decide(WM_QUERYENDSESSION, 0x1), Decision::Allow);
    }

    #[test]
    fn t_sessend_02_end_session_true_runs_the_shutdown() {
        // Arrange (wParam = TRUE) / Act / Assert
        assert_eq!(decide(WM_ENDSESSION, 1), Decision::Shutdown);
    }

    #[test]
    fn t_sessend_03_end_session_false_cancelled_does_nothing() {
        // Arrange (Boundary): 종료가 취소됐다 (wParam = FALSE)
        assert_eq!(decide(WM_ENDSESSION, 0), Decision::Ignore);
    }

    #[test]
    fn t_sessend_03b_unrelated_messages_are_ignored() {
        // Arrange: WM_CLOSE(0x10), WM_PAINT(0xF), 0
        for m in [0x0010, 0x000F, 0] {
            assert_eq!(decide(m, 1), Decision::Ignore, "msg {m:#x}");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn t_sessend_03c_not_ending_off_windows() {
        assert!(!is_ending());
    }
}
