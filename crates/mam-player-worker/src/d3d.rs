//! R2 렌더 경로: libmpv D3D11 렌더 API → BGRA8 렌더 타깃 텍스처 → staging 링(2개) → Map → 슬롯 메모리로 memcpy.
//! 이 memcpy(GPU → CPU 읽기 1회)가 R1(SW 렌더가 슬롯에 직접 쓰기)에는 없는 추가 비용이다.
//!
//! render_d3d11.h 규칙:
//! - 장치는 직접 만든다. `d3d11va` 가 무복사로 디코드하려면 멀티스레드 보호가 켜져 있어야 한다
//!   (`D3D11_CREATE_DEVICE_SINGLETHREADED` 면 `d3d11va-copy` 만 가능). VIDEO_SUPPORT·BGRA_SUPPORT 를 준다.
//! - mpv 는 즉시 컨텍스트로 그리고 상태를 복원하지 않는다. 이 모듈은 렌더 후 CopyResource/Map/Unmap 만 쓰고
//!   그 호출들은 파이프라인 상태(셰이더·렌더 타깃 바인딩)에 의존하지 않으므로 복원할 상태가 없다.
//!   mpv_render_* 가 도는 동안 다른 스레드가 즉시 컨텍스트를 쓰지 않도록 복사·Map 은 같은 렌더 스레드에서 렌더 직후에만 한다.

use super::mpv_api::{Api, Ptr, RP_D3D11_FBO, RenderParam};
use super::render::RenderTarget;
use std::ffi::c_void;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::Interface;

/// `mpv_d3d11_init_params` / `mpv_d3d11_fbo` (render_d3d11.h)
#[repr(C)]
pub struct InitParams {
    pub device: *mut c_void,
}
#[repr(C)]
struct Fbo {
    tex: *mut c_void,
}

struct Targets {
    w: u32,
    h: u32,
    rt: ID3D11Texture2D,
    staging: [ID3D11Texture2D; 2],
    next: usize,
}

pub struct D3d {
    pub device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    t: Option<Targets>,
}

// 렌더 스레드 한 곳에서만 즉시 컨텍스트를 쓴다 (Mutex 로 감싸 Shared 에 보관)
unsafe impl Send for D3d {}

impl D3d {
    pub fn new() -> Result<Self, String> {
        let levels: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let (mut device, mut ctx) = (None, None);
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
            .map_err(|e| format!("D3D11CreateDevice 실패: {e}"))?;
        }
        let (device, ctx) = (
            device.ok_or("D3D11 장치 없음")?,
            ctx.ok_or("D3D11 컨텍스트 없음")?,
        );
        // d3d11va 무복사 디코드의 전제: 디코더 스레드와 렌더 스레드가 같은 장치를 동시에 쓴다
        let mt: ID3D11Multithread = device
            .cast()
            .map_err(|e| format!("ID3D11Multithread 없음: {e}"))?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }
        Ok(Self {
            device,
            ctx,
            t: None,
        })
    }

    /// mpv 렌더 컨텍스트 생성 인자용 장치 포인터
    pub fn device_ptr(&self) -> *mut c_void {
        self.device.as_raw()
    }

    fn ensure(&mut self, w: u32, h: u32) -> Result<(), String> {
        if self.t.as_ref().is_some_and(|t| t.w == w && t.h == h) {
            return Ok(());
        }
        let mk = |usage, bind: u32, cpu: u32| -> Result<ID3D11Texture2D, String> {
            let d = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: usage,
                BindFlags: bind,
                CPUAccessFlags: cpu,
                MiscFlags: 0,
            };
            let mut t = None;
            unsafe { self.device.CreateTexture2D(&d, None, Some(&mut t)) }
                .map_err(|e| format!("CreateTexture2D {w}x{h} 실패: {e}"))?;
            t.ok_or_else(|| "텍스처 없음".to_string())
        };
        let rt = mk(D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0)?;
        let staging = [
            mk(D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32)?,
            mk(D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32)?,
        ];
        self.t = Some(Targets {
            w,
            h,
            rt,
            staging,
            next: 0,
        });
        Ok(())
    }

    /// 프레임 한 장: mpv 렌더 → 링의 다음 staging 으로 복사 → Map → 슬롯으로 memcpy. 반환: mpv 오류 코드(음수 = 실패)
    ///
    /// # Safety
    /// `ctx` 는 이 장치로 만든 유효한 D3D11 렌더 컨텍스트, `dst` 는 `stride*h` 바이트 이상 쓰기 가능
    pub unsafe fn render(&mut self, api: &Api, ctx: Ptr, dst: &RenderTarget) -> i32 {
        if let Err(e) = self.ensure(dst.w, dst.h) {
            eprintln!("[d3d] {e}");
            return -1;
        }
        let t = self.t.as_mut().expect("ensure 직후");
        let mut fbo = Fbo { tex: t.rt.as_raw() };
        let mut params = [
            RenderParam {
                typ: RP_D3D11_FBO,
                data: &mut fbo as *mut Fbo as *mut c_void,
            },
            RenderParam {
                typ: 0,
                data: std::ptr::null_mut(),
            },
        ];
        let rc = unsafe { (api.render_context_render)(ctx, params.as_mut_ptr()) };
        if rc < 0 {
            return rc;
        }
        // 링: 이번 프레임을 staging[i] 로 복사하고 곧바로 Map (GPU 완료를 기다림). 2개를 번갈아 써서 직전 Map 해제와 겹치지 않게 한다
        let i = t.next;
        t.next ^= 1;
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.ctx.CopyResource(&t.staging[i], &t.rt);
            if self
                .ctx
                .Map(&t.staging[i], 0, D3D11_MAP_READ, 0, Some(&mut m))
                .is_err()
            {
                return -2;
            }
            let row = (dst.w as usize * 4).min(dst.stride);
            for y in 0..dst.h as usize {
                std::ptr::copy_nonoverlapping(
                    (m.pData as *const u8).add(y * m.RowPitch as usize),
                    dst.ptr.add(y * dst.stride),
                    row,
                );
            }
            self.ctx.Unmap(&t.staging[i], 0);
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: External(GPU 의존 — 장치를 만들 수 없으면 SKIP) / Business rule(멀티스레드 보호·텍스처 재사용)

    #[test]
    fn t_player_worker_d3d_01_device_is_multithread_protected_and_targets_resize() {
        let Ok(mut d) = D3d::new() else {
            eprintln!("SKIP: D3D11 하드웨어 장치를 만들 수 없음");
            return;
        };
        let mt: ID3D11Multithread = d.device.cast().unwrap();
        assert!(
            unsafe { mt.GetMultithreadProtected() }.as_bool(),
            "d3d11va 무복사의 전제"
        );
        d.ensure(640, 360).unwrap();
        let first = d.t.as_ref().unwrap().rt.as_raw();
        d.ensure(640, 360).unwrap();
        assert_eq!(
            first,
            d.t.as_ref().unwrap().rt.as_raw(),
            "같은 크기면 텍스처를 재사용한다"
        );
        d.ensure(320, 180).unwrap();
        assert_eq!(
            (d.t.as_ref().unwrap().w, d.t.as_ref().unwrap().h),
            (320, 180)
        );
        assert!(d.ensure(0, 0).is_err(), "0 크기 텍스처는 오류");
    }
}
