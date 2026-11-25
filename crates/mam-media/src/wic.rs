//! Windows Imaging Component 디코딩 — HEIC/HEIF 의 마지막 폴백 (P1-5 결정, P2-3 D7).
//!
//! FFmpeg 로 디코딩하지 못한 HEIC 를 Windows 가 설치한 코덱(HEIF Image Extensions)으로 디코딩한다.
//! 코덱이 없으면 `CreateDecoderFromFilename` 이 실패하며, 이 경우 호출자는 두 폴백의 실패 사유를 함께 기록한다.
//! 원본은 읽기 전용으로만 열린다.

use std::path::Path;

use image::DynamicImage;

use crate::error::Result;

/// WIC 로 첫 프레임을 RGBA 로 디코딩한다. 크기가 `max_pixels` 를 넘으면 픽셀을 읽기 전에 거부한다.
#[cfg(windows)]
pub fn decode_with_wic(path: &Path, max_pixels: u64) -> Result<DynamicImage> {
    imp::decode(path, max_pixels)
}

#[cfg(not(windows))]
pub fn decode_with_wic(_path: &Path, _max_pixels: u64) -> Result<DynamicImage> {
    Err(crate::error::MediaError::Unsupported(
        "WIC is only available on Windows",
    ))
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use image::{DynamicImage, RgbaImage};
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
        WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::core::PCWSTR;

    use crate::error::{MediaError, Result};

    /// 스레드의 COM 초기화를 균형 있게 해제한다 (이미 다른 모드로 초기화된 스레드는 건드리지 않는다).
    struct ComGuard(bool);

    impl ComGuard {
        fn init() -> Self {
            // S_OK / S_FALSE 면 우리가 올린 참조이므로 해제한다. RPC_E_CHANGED_MODE 는 이미 STA 로 초기화된 스레드다
            Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn wic_err(stage: &str, e: windows::core::Error) -> MediaError {
        MediaError::Decode(format!("wic {stage}: {e}"))
    }

    pub fn decode(path: &Path, max_pixels: u64) -> Result<DynamicImage> {
        let _com = ComGuard::init();
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| wic_err("factory", e))?;
            let decoder = factory
                .CreateDecoderFromFilename(
                    PCWSTR(wide.as_ptr()),
                    None,
                    GENERIC_READ,
                    WICDecodeMetadataCacheOnDemand,
                )
                .map_err(|e| wic_err("decoder (codec not installed?)", e))?;
            let frame = decoder.GetFrame(0).map_err(|e| wic_err("frame", e))?;
            let (mut w, mut h) = (0u32, 0u32);
            frame
                .GetSize(&mut w, &mut h)
                .map_err(|e| wic_err("size", e))?;
            if w == 0 || h == 0 || u64::from(w) * u64::from(h) > max_pixels {
                return Err(MediaError::ImageTooLarge {
                    width: w,
                    height: h,
                    max_pixels,
                });
            }
            let converter = factory
                .CreateFormatConverter()
                .map_err(|e| wic_err("converter", e))?;
            converter
                .Initialize(
                    &frame,
                    &GUID_WICPixelFormat32bppRGBA,
                    WICBitmapDitherTypeNone,
                    None,
                    0.0,
                    WICBitmapPaletteTypeCustom,
                )
                .map_err(|e| wic_err("convert", e))?;
            let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
            converter
                .CopyPixels(std::ptr::null(), w * 4, &mut pixels)
                .map_err(|e| wic_err("pixels", e))?;
            RgbaImage::from_raw(w, h, pixels)
                .map(DynamicImage::ImageRgba8)
                .ok_or_else(|| MediaError::Decode("wic: buffer size mismatch".into()))
        }
    }
}
