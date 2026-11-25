//! 렌더 스레드와 공유 상태.
//! libmpv 스레드 규칙 (render.h): `mpv_render_*` 는 한 번에 하나만, 업데이트 콜백 안에서는 호출 금지,
//! 렌더 스레드는 "안전하지 않은" mpv API(속성 조회 등)를 호출하거나 그 완료를 기다리지 않는다.
//! → 렌더 스레드는 render_* 와 자체 상태만 만진다. time-pos 같은 속성은 이벤트 스레드가 조회한다.

use super::mpv_api::{
    Api, Ptr, RP_API_TYPE, RP_SW_FORMAT, RP_SW_POINTER, RP_SW_SIZE, RP_SW_STRIDE, RenderParam,
    SendPtr, UPDATE_FRAME,
};
use mam_player::protocol::Message;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// 렌더 결과를 쓸 메모리 한 면: 첫 픽셀 포인터 + 줄 간격(바이트) + 크기. 픽셀은 bgr0 (4바이트)
#[derive(Clone, Copy)]
pub struct RenderTarget {
    pub ptr: *mut u8,
    pub stride: usize,
    pub w: u32,
    pub h: u32,
}
unsafe impl Send for RenderTarget {}

impl RenderTarget {
    pub fn len(&self) -> usize {
        self.stride * self.h as usize
    }
}

/// 렌더 스레드 ↔ 나머지 스레드 공유 상태
pub struct Shared {
    wake: Mutex<bool>,
    cv: Condvar,
    pub stop: AtomicBool,
    /// 연결된 슬롯 묶음 (없으면 프레임을 렌더 없이 소비한다)
    pub bank: Mutex<Option<Arc<super::bank::Bank>>>,
    pub rendered: AtomicU64,
    pub skipped: AtomicU64,
    pub ignored: AtomicU64,
    /// 새 슬롯 연결·렌더 재개 직후 한 번 강제로 다시 그린다 (정지 중에도 현재 프레임을 한 프레임 안에 게시)
    pub redraw: AtomicBool,
    /// 보이지 않는다 (창 최소화·보기 숨김) — 렌더·게시를 멈춘다
    pub render_active: AtomicBool,
    /// 내부 전환 중(구간 재열기·길이 탐색) — 중간 프레임을 게시하지 않는다
    pub hold: AtomicBool,
    /// 화면에 낼 프레임이 있는가 (열린 미디어의 첫 재시작 이후). 없으면 강제 다시 그리기를 하지 않는다
    pub has_video: AtomicBool,
    /// R2 모드의 D3D11 렌더러 (None = R1)
    pub d3d: Mutex<Option<super::d3d::D3d>>,
    /// 뒤로 이동 프레임 캐시 (장GOP 에서만 켠다)
    pub cache: Mutex<Option<super::cache::FrameCache>>,
    /// 캐시로 응답한 뒤 mpv 가 따라잡느라 실제로 수행하는 frame-back-step 의 남은 횟수 — 그 결과 프레임은 내보내지 않는다
    pub suppress: AtomicU32,
    pub suppress_until: Mutex<Instant>,
}

impl Shared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            wake: Mutex::new(false),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            bank: Mutex::new(None),
            rendered: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
            ignored: AtomicU64::new(0),
            redraw: AtomicBool::new(false),
            render_active: AtomicBool::new(true),
            hold: AtomicBool::new(false),
            has_video: AtomicBool::new(false),
            d3d: Mutex::new(None),
            cache: Mutex::new(None),
            suppress: AtomicU32::new(0),
            suppress_until: Mutex::new(Instant::now()),
        })
    }

    pub fn notify(&self) {
        *self.wake.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cv.notify_one();
    }

    /// 깨워질 때까지(또는 `timeout`) 대기. 깨워졌으면 true
    fn wait(&self, timeout: Duration) -> bool {
        let g = self.wake.lock().unwrap_or_else(|p| p.into_inner());
        let (mut g, _) = self
            .cv
            .wait_timeout_while(g, timeout, |w| !*w)
            .unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut *g)
    }
}

/// mpv 업데이트 콜백 — 어떤 mpv API 도 호출하지 않고 신호만 보낸다
pub unsafe extern "C" fn on_update(ctx: *mut c_void) {
    let shared = unsafe { &*(ctx as *const Shared) };
    shared.notify();
}

/// SW 렌더 컨텍스트 생성 (`vo=libmpv` 로 초기화된 핸들 필요)
pub fn create_context(api: &Api, mpv: Ptr) -> Result<SendPtr, String> {
    let mut ctx: Ptr = std::ptr::null_mut();
    let mut params = [
        RenderParam {
            typ: RP_API_TYPE,
            data: c"sw".as_ptr() as *mut c_void,
        },
        RenderParam {
            typ: 0,
            data: std::ptr::null_mut(),
        },
    ];
    let r = unsafe { (api.render_context_create)(&mut ctx, mpv, params.as_mut_ptr()) };
    if r < 0 || ctx.is_null() {
        Err(format!("render context create failed ({r})"))
    } else {
        Ok(SendPtr(ctx))
    }
}

/// D3D11 렌더 컨텍스트 생성 (R2). `device` 는 직접 만든 멀티스레드 보호 장치
pub fn create_context_d3d11(api: &Api, mpv: Ptr, device: *mut c_void) -> Result<SendPtr, String> {
    let mut ctx: Ptr = std::ptr::null_mut();
    let mut init = super::d3d::InitParams { device };
    let mut params = [
        RenderParam {
            typ: RP_API_TYPE,
            data: c"d3d11".as_ptr() as *mut c_void,
        },
        RenderParam {
            typ: super::mpv_api::RP_D3D11_INIT_PARAMS,
            data: &mut init as *mut _ as *mut c_void,
        },
        RenderParam {
            typ: 0,
            data: std::ptr::null_mut(),
        },
    ];
    let r = unsafe { (api.render_context_create)(&mut ctx, mpv, params.as_mut_ptr()) };
    if r < 0 || ctx.is_null() {
        Err(format!("D3D11 render context create failed ({r})"))
    } else {
        Ok(SendPtr(ctx))
    }
}

/// 렌더 경로 선택: R2 면 D3D11 → staging → 슬롯 복사, 아니면 R1(SW 가 슬롯에 직접 렌더)
pub(super) unsafe fn render_target(api: &Api, ctx: Ptr, shared: &Shared, t: &RenderTarget) -> i32 {
    match shared
        .d3d
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_mut()
    {
        Some(d) => unsafe { d.render(api, ctx, t) },
        None => unsafe { render_once(api, ctx, t) },
    }
}

/// 프레임 한 장을 대상 버퍼에 렌더한다. 반환: mpv 오류 코드(음수면 실패)
pub(super) unsafe fn render_once(api: &Api, ctx: Ptr, t: &RenderTarget) -> i32 {
    let mut size = [t.w as i32, t.h as i32];
    let mut stride = t.stride;
    let mut params = [
        RenderParam {
            typ: RP_SW_SIZE,
            data: size.as_mut_ptr() as *mut c_void,
        },
        RenderParam {
            typ: RP_SW_FORMAT,
            data: c"bgr0".as_ptr() as *mut c_void,
        },
        RenderParam {
            typ: RP_SW_STRIDE,
            data: &mut stride as *mut usize as *mut c_void,
        },
        RenderParam {
            typ: RP_SW_POINTER,
            data: t.ptr as *mut c_void,
        },
        RenderParam {
            typ: 0,
            data: std::ptr::null_mut(),
        },
    ];
    unsafe { (api.render_context_render)(ctx, params.as_mut_ptr()) }
}

/// 프레임을 그리지 않고 "그린 것으로" 소비한다 (SKIP_RENDERING — 대상 표면 불필요). mpv 큐가 막히지 않게 한다
pub(super) unsafe fn skip_render(api: &Api, ctx: Ptr) {
    let mut skip: i32 = 1;
    let mut params = [
        RenderParam {
            typ: super::mpv_api::RP_SKIP_RENDERING,
            data: &mut skip as *mut i32 as *mut c_void,
        },
        RenderParam {
            typ: 0,
            data: std::ptr::null_mut(),
        },
    ];
    unsafe {
        (api.render_context_render)(ctx, params.as_mut_ptr());
        (api.render_context_report_swap)(ctx);
    }
}

/// 렌더 스레드 시작. 업데이트 신호 → `update` → FRAME 플래그면 렌더 → 슬롯 게시
pub fn spawn_render_thread(
    api: Arc<Api>,
    ctx: SendPtr,
    shared: Arc<Shared>,
    tx: Sender<Message>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let ctx = ctx; // 2024 에디션은 필드(.0)만 캡처하므로 SendPtr 전체를 옮긴다
        while !shared.stop.load(Ordering::Acquire) {
            // 신호가 없어도 50ms 마다 한 번 update 를 호출해 놓친 콜백을 보정한다 (update 는 render 스레드 호출이 허용됨)
            let _ = shared.wait(Duration::from_millis(50));
            let flags = unsafe { (api.render_context_update)(ctx.0) };
            let forced = shared.redraw.swap(false, Ordering::AcqRel)
                && shared.has_video.load(Ordering::Acquire);
            if flags & UPDATE_FRAME == 0 && !forced {
                continue;
            }
            let bank = shared
                .bank
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            match bank {
                Some(b) => super::bank::render_into_bank(&api, ctx.0, &b, &shared, &tx),
                // 슬롯이 없으면 SW 표면이 없으므로 프레임을 렌더 없이 소비한다
                None => unsafe { skip_render(&api, ctx.0) },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Concurrency(신호 유실 없음)

    #[test]
    fn t_player_worker_render_01_wake_signal_is_not_lost() {
        // 대기 시작 전에 온 신호도 소비된다 (콜백이 먼저 오고 스레드가 나중에 대기해도 놓치지 않는다)
        let s = Shared::new();
        s.notify();
        assert!(s.wait(Duration::from_millis(10)));
        assert!(
            !s.wait(Duration::from_millis(10)),
            "신호는 한 번만 소비된다"
        );
    }
}
