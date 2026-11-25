//! 호스트가 보낸 파일 매핑 핸들(WebView2 SharedBuffer)을 `MapViewOfFile` 해 만든 슬롯 묶음.
//! SW 렌더(R1)는 각 슬롯 메모리에 `bgr0` 를 직접 쓴다 (복사 없음), R2 는 staging 에서 슬롯으로 한 번 복사한다.
//! 렌더 스레드는 `Arc<Bank>` 를 잡고 렌더하므로, 교체돼도 마지막 렌더가 끝나야 언맵된다.

use super::mpv_api::{Api, Ptr, RP_SKIP_RENDERING, RenderParam};
use super::render::{RenderTarget, Shared};
use mam_player::protocol::{Message, SLOT_COUNT, SlotTable};
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, UnmapViewOfFile};

/// render.h 권장 정렬 — 포인터와 stride 둘 다 64 의 배수
const ALIGN: usize = 64;

pub struct Bank {
    pub epoch: u64,
    pub targets: Vec<RenderTarget>,
    /// 워커 프로세스 안의 복제된 매핑 핸들 (Drop 에서 닫는다)
    handles: Vec<isize>,
    pub table: Mutex<SlotTable>,
}

unsafe impl Send for Bank {}
unsafe impl Sync for Bank {}

/// 인자 검증 (순수 — 단위 테스트 대상). 외부 입력은 악의적이라고 본다
pub fn validate(w: u32, h: u32, stride: u32, size: u64, handles: usize) -> Result<(), String> {
    if handles != SLOT_COUNT as usize {
        return Err(format!("invalid slot count: {handles}"));
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err(format!("invalid render size: {w}x{h}"));
    }
    if !(stride as usize).is_multiple_of(ALIGN) {
        return Err(format!("stride {stride} is not a multiple of {ALIGN}"));
    }
    if (stride as usize) < w as usize * 4 {
        return Err(format!("stride {stride} < width*4"));
    }
    if size < stride as u64 * h as u64 {
        return Err(format!("buffer size {size} < stride*h"));
    }
    Ok(())
}

impl Bank {
    pub fn attach(epoch: u64, w: u32, h: u32, stride: u32, size: u64, handles: &[u64]) -> Result<Self, String> {
        if let Err(e) = validate(w, h, stride, size, handles.len()) {
            // 이미 넘겨받은 핸들은 닫는다 (누수 방지)
            for &v in handles {
                unsafe {
                    let _ = CloseHandle(HANDLE(v as isize as *mut _));
                }
            }
            return Err(e);
        }
        // 부분 실패에도 Drop 이 정리하도록 먼저 빈 뱅크를 만든다
        let mut bank = Bank { epoch, targets: vec![], handles: handles.iter().map(|&v| v as isize).collect(), table: Mutex::new(SlotTable::new(epoch, handles.len())) };
        for &v in handles {
            let view = unsafe { MapViewOfFile(HANDLE(v as isize as *mut _), FILE_MAP_WRITE, 0, 0, 0) };
            if view.Value.is_null() {
                return Err(format!("MapViewOfFile failed: {}", std::io::Error::last_os_error()));
            }
            let ptr = view.Value as *mut u8;
            if !(ptr as usize).is_multiple_of(ALIGN) {
                unsafe {
                    let _ = UnmapViewOfFile(view);
                }
                return Err("mapped view is not 64-byte aligned".into());
            }
            bank.targets.push(RenderTarget { ptr, stride: stride as usize, w, h });
        }
        Ok(bank)
    }
}

/// 현재 시각을 epoch ms (소수점 포함) 로 — JS 의 `performance.timeOrigin + performance.now()` 와 비교하는 기준
pub fn now_epoch_ms() -> f64 {
    let ft = unsafe { windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime() };
    let ticks = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    ticks as f64 / 10_000.0 - 11_644_473_600_000.0
}

/// FRAME 갱신이 왔을 때 슬롯 하나를 잡아 렌더한다. FREE 슬롯이 없으면 기다리지 않고 이번 프레임을 렌더 없이 소비(건너뜀)한다.
/// 보이지 않거나(`render_active == false`) 내부 전환 중(`hold`)이면 렌더하지 않고 소비만 한다
pub fn render_into_bank(api: &Api, ctx: Ptr, bank: &Bank, shared: &Shared, tx: &Sender<Message>) {
    if !shared.render_active.load(Ordering::Acquire) || shared.hold.load(Ordering::Acquire) {
        unsafe { super::render::skip_render(api, ctx) };
        return;
    }
    // 캐시로 이미 응답한 뒤 mpv 가 따라잡는 중인 프레임은 화면에 내보내지 않는다 (옛 프레임이 다시 보이는 것 방지)
    if shared.suppress.load(Ordering::Acquire) > 0 {
        if Instant::now() < *shared.suppress_until.lock().unwrap_or_else(|p| p.into_inner()) {
            unsafe { super::render::skip_render(api, ctx) };
            return;
        }
        shared.suppress.store(0, Ordering::Release); // 안전장치: 복귀 신호가 오지 않아도 영구히 막지 않는다
    }
    let slot = bank.table.lock().unwrap_or_else(|p| p.into_inner()).acquire();
    let Some(slot) = slot else {
        // SKIP_RENDERING: 프레임을 "그린 것으로" 처리해 mpv 큐가 막히지 않게 한다 (대상 표면 불필요)
        let mut skip: i32 = 1;
        let mut params = [RenderParam { typ: RP_SKIP_RENDERING, data: &mut skip as *mut i32 as *mut c_void }, RenderParam { typ: 0, data: std::ptr::null_mut() }];
        unsafe {
            (api.render_context_render)(ctx, params.as_mut_ptr());
            (api.render_context_report_swap)(ctx);
        }
        shared.skipped.fetch_add(1, Ordering::Relaxed);
        return;
    };
    let t = bank.targets[slot];
    let rc = unsafe { super::render::render_target(api, ctx, shared, &t) };
    if rc < 0 {
        bank.table.lock().unwrap_or_else(|p| p.into_inner()).abort(slot);
        let _ = tx.send(Message::Error { code: mam_player::protocol::WorkerErrorCode::Internal, msg: format!("render failed ({rc})") });
        return;
    }
    unsafe { (api.render_context_report_swap)(ctx) };
    let done = now_epoch_ms();
    let Some(seq) = bank.table.lock().unwrap_or_else(|p| p.into_inner()).publish(slot) else { return };
    if let Some(c) = shared.cache.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
        // SAFETY: 방금 이 스레드가 쓴 슬롯 메모리 (JS 는 publish 후에야 읽는다 — 호스트 메시지 이후)
        let data = unsafe { std::slice::from_raw_parts(t.ptr, t.stride * t.h as usize) };
        c.push(super::cache::hash_pixels(data, t.stride, t.w, t.h), t.w, t.h, t.stride, data);
    }
    shared.rendered.fetch_add(1, Ordering::Relaxed);
    let _ = tx.send(Message::FrameReady { epoch: bank.epoch, slot: slot as u32, seq, w: t.w, h: t.h, stride: t.stride as u32, render_done_ms: done });
}

impl Drop for Bank {
    fn drop(&mut self) {
        for t in &self.targets {
            unsafe {
                let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: t.ptr as *mut _ });
            }
        }
        for &h in &self.handles {
            unsafe {
                let _ = CloseHandle(HANDLE(h as *mut _));
            }
        }
    }
}

use std::time::Instant;

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
    use windows::core::PCWSTR;

    // 분류: Validation / Boundary / Happy(실제 매핑)

    #[test]
    fn t_player_worker_bank_01_validate_rejects_bad_geometry() {
        assert!(validate(1280, 720, 5120, 5120 * 720, 3).is_ok());
        assert!(validate(0, 720, 5120, 1 << 20, 3).is_err(), "너비 0");
        assert!(validate(1280, 720, 5000, 5000 * 720, 3).is_err(), "stride 정렬");
        assert!(validate(1280, 720, 4992, 4992 * 720, 3).is_err(), "stride < w*4");
        assert!(validate(1280, 720, 5120, 5120 * 720 - 1, 3).is_err(), "버퍼가 작음");
        assert!(validate(1280, 720, 5120, 5120 * 720, 2).is_err(), "슬롯은 정확히 3개");
        assert!(validate(1280, 720, 5120, 5120 * 720, 9).is_err());
        assert!(validate(20000, 720, 80000, 1 << 30, 3).is_err(), "너무 큼");
    }

    #[test]
    fn t_player_worker_bank_02_maps_real_section_aligned_and_writable() {
        let (w, h, stride) = (640u32, 360u32, 2560u32);
        let size = stride as u64 * h as u64;
        let handles: Vec<u64> = (0..3)
            .map(|_| unsafe { CreateFileMappingW(HANDLE(-1isize as *mut _), None, PAGE_READWRITE, (size >> 32) as u32, size as u32, PCWSTR::null()).unwrap().0 as u64 })
            .collect();
        let bank = Bank::attach(7, w, h, stride, size, &handles).expect("attach");
        assert_eq!(bank.targets.len(), 3);
        for t in &bank.targets {
            assert_eq!(t.ptr as usize % 64, 0);
            unsafe { *t.ptr.add(t.len() - 1) = 0xAB };
            assert_eq!(unsafe { *t.ptr.add(t.len() - 1) }, 0xAB);
        }
        assert_eq!(bank.table.lock().unwrap().epoch(), 7);
    }

    #[test]
    fn t_player_worker_bank_03_invalid_handle_is_error_not_crash() {
        assert!(Bank::attach(1, 640, 360, 2560, 2560 * 360, &[0xdead_beef, 0xdead_bef0, 0xdead_bef1]).is_err());
    }
}
