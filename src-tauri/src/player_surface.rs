//! 플레이어 프레임 표면 — WebView2 SharedBuffer 3개를 만들어 JS 에 게시하고, 프레임 알림을 `PostWebMessageAsJson` 으로 보낸다 (계약 §3, §4).
//! JS 의 슬롯 반환은 이 모듈이 받지 않는다 — `chrome.webview.postMessage(object)` 는 wry 의 먼저 등록된 핸들러가 오류를 반환해
//! 뒤 핸들러에 닿지 않으므로, Tauri command `player_release_slots`(`player_commands.rs`)로 받는다.
//!
//! 규칙(계약 §5.4): WebView2 호출은 **UI 스레드에서만** 한다 (`with_webview` 클로저). 감독자 스레드는 `create_bank` 에서
//! UI 스레드 결과를 채널로 최대 2초 기다리고, 그 밖의 호출(`retire_bank`·`post_frame`)은 큐에 넣고 즉시 돌아온다.

#[cfg(windows)]
pub use imp::TauriSurface;

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::mpsc;
    use std::sync::Mutex;
    use std::time::Duration;

    use mam_player::host::{BankAttach, FrameSurface, RawProcessHandle, SurfaceError};
    use mam_player::protocol::{BufferInfo, FrameMsgOut, SLOT_COUNT};
    use tauri::{AppHandle, Manager};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, ICoreWebView2, ICoreWebView2_2, ICoreWebView2_17, ICoreWebView2Environment12, ICoreWebView2SharedBuffer,
    };
    use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::core::{HSTRING, Interface};

    /// UI 스레드의 응답 대기 상한
    const UI_WAIT: Duration = Duration::from_secs(2);

    thread_local! {
        /// UI 스레드가 쥔 SharedBuffer (epoch 별) — COM 객체라 이 스레드에서만 만지고 닫는다
        static BANKS: RefCell<HashMap<u64, Vec<ICoreWebView2SharedBuffer>>> = RefCell::new(HashMap::new());
    }

    pub struct TauriSurface {
        app: AppHandle,
        supported: Mutex<Option<bool>>,
    }

    impl TauriSurface {
        pub fn new(app: AppHandle) -> Self {
            Self { app, supported: Mutex::new(None) }
        }

        fn run_ui<T: Send + 'static>(&self, f: impl FnOnce(&ICoreWebView2) -> Result<T, String> + Send + 'static) -> Result<T, SurfaceError> {
            let window = self.app.get_webview_window("main").ok_or_else(|| SurfaceError::Failed("main window not found".into()))?;
            let (tx, rx) = mpsc::channel();
            window
                .with_webview(move |pw| {
                    // SAFETY: with_webview 클로저는 UI 스레드에서 실행되고 controller 는 살아 있는 COM 포인터다
                    let r = unsafe { pw.controller().CoreWebView2() }.map_err(|e| e.to_string()).and_then(|wv| f(&wv));
                    let _ = tx.send(r);
                })
                .map_err(|e| SurfaceError::Failed(e.to_string()))?;
            rx.recv_timeout(UI_WAIT).map_err(|_| SurfaceError::Timeout)?.map_err(SurfaceError::Failed)
        }

    }

    /// UI 스레드 — 버퍼 3개를 만들어 읽기 전용으로 게시하고 워커로 핸들을 복제한다
    fn create_on_ui(wv: &ICoreWebView2, epoch: u64, w: u32, h: u32, worker: RawProcessHandle) -> Result<BankAttach, String> {
        let wv2: ICoreWebView2_2 = wv.cast().map_err(|e| format!("ICoreWebView2_2: {e}"))?;
        // SAFETY: UI 스레드의 살아 있는 COM 객체들
        let env = unsafe { wv2.Environment() }.map_err(|e| format!("Environment: {e}"))?;
        let env12: ICoreWebView2Environment12 = env.cast().map_err(|e| format!("Environment12 (runtime too old?): {e}"))?;
        let wv17: ICoreWebView2_17 = wv.cast().map_err(|e| format!("ICoreWebView2_17 (runtime too old?): {e}"))?;
        let first = BufferInfo::new(epoch, 0, w, h);
        let mut buffers = Vec::with_capacity(SLOT_COUNT as usize);
        let mut handles = Vec::with_capacity(SLOT_COUNT as usize);
        for slot in 0..SLOT_COUNT {
            let sb = unsafe { env12.CreateSharedBuffer(first.size) }.map_err(|e| format!("CreateSharedBuffer: {e}"))?;
            let info = serde_json::to_string(&BufferInfo::new(epoch, slot, w, h)).map_err(|e| e.to_string())?;
            unsafe { wv17.PostSharedBufferToScript(&sb, COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, &HSTRING::from(info)) }.map_err(|e| format!("PostSharedBufferToScript: {e}"))?;
            let mut src = HANDLE::default();
            unsafe { sb.FileMappingHandle(&mut src) }.map_err(|e| format!("FileMappingHandle: {e}"))?;
            let mut dst = HANDLE::default();
            unsafe { DuplicateHandle(GetCurrentProcess(), src, HANDLE(worker as *mut c_void), &mut dst, 0, false, DUPLICATE_SAME_ACCESS) }.map_err(|e| format!("DuplicateHandle: {e}"))?;
            handles.push(dst.0 as usize as u64);
            buffers.push(sb);
        }
        BANKS.with(|b| b.borrow_mut().insert(epoch, buffers));
        Ok(BankAttach { epoch, w, h, stride: first.stride, size: first.size, handles })
    }

    impl FrameSurface for TauriSurface {
        fn create_bank(&self, epoch: u64, w: u32, h: u32, worker_process: RawProcessHandle) -> Result<BankAttach, SurfaceError> {
            self.run_ui(move |wv| create_on_ui(wv, epoch, w, h, worker_process))
        }

        fn retire_bank(&self, epoch: u64) {
            // 비차단: UI 스레드 큐에 넣고 돌아온다. 호스트가 쥔 참조만 닫는다 (JS·워커가 쥔 뷰는 각자 해제될 때까지 유효)
            if let Some(window) = self.app.get_webview_window("main") {
                let _ = window.with_webview(move |_| {
                    BANKS.with(|b| {
                        if let Some(list) = b.borrow_mut().remove(&epoch) {
                            for sb in list {
                                // SAFETY: UI 스레드에서 만든 객체를 같은 스레드에서 닫는다
                                unsafe {
                                    let _ = sb.Close();
                                }
                            }
                        }
                    });
                });
            }
        }

        fn post_frame(&self, msg: &FrameMsgOut) {
            let Ok(json) = serde_json::to_string(msg) else { return };
            if let Some(window) = self.app.get_webview_window("main") {
                let _ = window.with_webview(move |pw| {
                    // SAFETY: UI 스레드
                    unsafe {
                        if let Ok(wv) = pw.controller().CoreWebView2() {
                            let _ = wv.PostWebMessageAsJson(&HSTRING::from(json));
                        }
                    }
                });
            }
        }

        fn capabilities_ok(&self) -> bool {
            if let Some(v) = *self.supported.lock().unwrap_or_else(|p| p.into_inner()) {
                return v;
            }
            // 처음 한 번 UI 스레드에서 캐스트를 확인한다 (감독자·명령 스레드에서 부른다 — UI 스레드에서 부르면 안 된다)
            let r = self.run_ui(|wv| {
                let wv2: ICoreWebView2_2 = wv.cast().map_err(|e| e.to_string())?;
                let env = unsafe { wv2.Environment() }.map_err(|e| e.to_string())?;
                env.cast::<ICoreWebView2Environment12>().map_err(|e| e.to_string())?;
                wv.cast::<ICoreWebView2_17>().map_err(|e| e.to_string())?;
                Ok(())
            });
            match r {
                Ok(()) => {
                    *self.supported.lock().unwrap_or_else(|p| p.into_inner()) = Some(true);
                    true
                }
                // 창이 아직 없거나 UI 가 바쁘면 캐시하지 않는다 (다음에 다시 본다)
                Err(SurfaceError::Timeout) | Err(SurfaceError::Failed(_)) if self.app.get_webview_window("main").is_none() => false,
                Err(e) => {
                    tracing::info!(%e, "player: WebView2 shared buffers are not supported");
                    *self.supported.lock().unwrap_or_else(|p| p.into_inner()) = Some(false);
                    false
                }
            }
        }
    }
}
