//! 시스템(Windows) 음량 조정 command `system_volume_step { delta }` -> 변경 후 0..1 (0.01 단위).
//! 기본 출력 장치(eRender/eMultimedia)의 마스터 음량을 **상대 변경 1회**로 바꾼다(읽기-쓰기 경합은 이 호출 안에서만).
//! PC 전체 음량이라 앱 음량으로 대체하지 않는다 - 장치 없음·COM 실패는 `audio_device_unavailable`(상세는 로그만).
//! 엔드포인트는 [`VolumeEndpoint`] trait 뒤에 두어 테스트가 실제 시스템 음량을 건드리지 않는다.

use crate::error::{CommandError, ErrorCode};

/// 기본 출력 장치 마스터 음량 접근 (0.0..=1.0 scalar). 실패 원인은 로그용 문자열
pub trait VolumeEndpoint {
    fn get(&self) -> Result<f32, String>;
    fn set(&self, scalar: f32) -> Result<(), String>;
}

fn unavailable(cause: &dyn std::fmt::Display) -> CommandError {
    CommandError::logged(
        ErrorCode::AudioDeviceUnavailable,
        "audio device is not available",
        cause,
    )
}

/// `delta` 검증(유한, -1..=1) -> 현재값 + delta 를 0..1 로 자르고 0.01 단위로 반올림해 설정 -> 설정한 값
pub fn step_volume(endpoint: &dyn VolumeEndpoint, delta: f64) -> Result<f64, CommandError> {
    if !delta.is_finite() || !(-1.0..=1.0).contains(&delta) {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "delta must be a finite number within -1..1",
        ));
    }
    let current = endpoint.get().map_err(|e| unavailable(&e))?;
    let next = ((f64::from(current) + delta).clamp(0.0, 1.0) * 100.0).round() / 100.0;
    endpoint.set(next as f32).map_err(|e| unavailable(&e))?;
    Ok(next)
}

#[cfg(windows)]
mod core_audio {
    use super::VolumeEndpoint;
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };

    /// 스레드의 COM 아파트 수명. 이 스레드가 초기화에 성공했을 때만(S_OK/S_FALSE) 해제한다.
    /// 이미 다른 모델로 초기화돼 있으면(RPC_E_CHANGED_MODE) 그대로 쓰고 해제하지 않는다
    pub struct ComScope {
        initialized: bool,
    }

    impl ComScope {
        pub fn enter() -> Self {
            // SAFETY: 이 스레드에서 짝이 맞는 CoUninitialize 를 Drop 에서 부른다
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            Self {
                initialized: hr.is_ok(),
            }
        }
    }

    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.initialized {
                // SAFETY: enter 에서 성공한 초기화와 짝
                unsafe { CoUninitialize() };
            }
        }
    }

    pub struct DefaultRenderVolume(IAudioEndpointVolume);

    impl DefaultRenderVolume {
        /// 호출 스레드에 [`ComScope`] 가 살아 있어야 하고, 이 값은 그 scope 보다 먼저 해제돼야 한다
        pub fn open() -> Result<Self, String> {
            // SAFETY: COM 호출 - 호출자가 COM 을 초기화한 스레드다
            unsafe {
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                        .map_err(|e| e.to_string())?;
                let device = enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)
                    .map_err(|e| e.to_string())?;
                let volume = device
                    .Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                    .map_err(|e| e.to_string())?;
                Ok(Self(volume))
            }
        }
    }

    impl VolumeEndpoint for DefaultRenderVolume {
        fn get(&self) -> Result<f32, String> {
            // SAFETY: 살아 있는 COM 인터페이스
            unsafe { self.0.GetMasterVolumeLevelScalar() }.map_err(|e| e.to_string())
        }

        fn set(&self, scalar: f32) -> Result<(), String> {
            // SAFETY: 이벤트 컨텍스트 GUID 는 쓰지 않는다(null)
            unsafe { self.0.SetMasterVolumeLevelScalar(scalar, std::ptr::null()) }
                .map_err(|e| e.to_string())
        }
    }
}

/// 블로킹 스레드에서 실행한다 (COM 초기화 포함)
#[cfg(windows)]
fn step_default_device(delta: f64) -> Result<f64, CommandError> {
    let _com = core_audio::ComScope::enter(); // 선언 순서상 endpoint 보다 늦게 해제된다
    let endpoint = core_audio::DefaultRenderVolume::open().map_err(|e| unavailable(&e))?;
    step_volume(&endpoint, delta)
}

#[cfg(not(windows))]
fn step_default_device(delta: f64) -> Result<f64, CommandError> {
    step_volume(&NoDevice, delta)
}

#[cfg(not(windows))]
struct NoDevice;

#[cfg(not(windows))]
impl VolumeEndpoint for NoDevice {
    fn get(&self) -> Result<f32, String> {
        Err("unsupported platform".into())
    }
    fn set(&self, _: f32) -> Result<(), String> {
        Err("unsupported platform".into())
    }
}

/// 시스템 음량을 `delta`(예: ±0.05) 만큼 바꾸고 변경 후 값(0..1, 0.01 단위)을 돌려준다.
/// 오류: `invalid_input`(delta 가 유한하지 않거나 -1..1 밖), `audio_device_unavailable`(장치 없음·COM 실패)
#[tauri::command]
pub async fn system_volume_step(delta: f64) -> Result<f64, CommandError> {
    if !delta.is_finite() || !(-1.0..=1.0).contains(&delta) {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "delta must be a finite number within -1..1",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || step_default_device(delta))
        .await
        .map_err(|e| CommandError::logged(ErrorCode::Internal, "internal error", &e))?
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary (0·1 clamp, 반올림) / Validation (delta) / External (장치 없음)
    use std::cell::Cell;

    use super::*;

    struct Stub {
        level: Cell<f32>,
        fail_get: bool,
        fail_set: bool,
        sets: Cell<u32>,
    }

    fn stub(level: f32) -> Stub {
        Stub {
            level: Cell::new(level),
            fail_get: false,
            fail_set: false,
            sets: Cell::new(0),
        }
    }

    impl VolumeEndpoint for Stub {
        fn get(&self) -> Result<f32, String> {
            if self.fail_get {
                Err("no endpoint".into())
            } else {
                Ok(self.level.get())
            }
        }
        fn set(&self, scalar: f32) -> Result<(), String> {
            if self.fail_set {
                return Err("set failed".into());
            }
            self.sets.set(self.sets.get() + 1);
            self.level.set(scalar);
            Ok(())
        }
    }

    #[test]
    fn t_sysvol_01_steps_up_and_down_and_returns_the_new_scalar() {
        let ep = stub(0.50);
        assert_eq!(step_volume(&ep, 0.05).unwrap(), 0.55);
        assert_eq!(ep.level.get(), 0.55);
        assert_eq!(step_volume(&ep, -0.10).unwrap(), 0.45);
        assert_eq!(ep.level.get(), 0.45);
    }

    #[test]
    fn t_sysvol_02_clamps_at_zero_and_one() {
        let low = stub(0.02);
        assert_eq!(step_volume(&low, -0.05).unwrap(), 0.0);
        assert_eq!(low.level.get(), 0.0);
        let high = stub(0.98);
        assert_eq!(step_volume(&high, 0.05).unwrap(), 1.0);
        assert_eq!(high.level.get(), 1.0);
        // 경계 delta 도 허용 (-1 / 1)
        assert_eq!(step_volume(&stub(0.4), 1.0).unwrap(), 1.0);
        assert_eq!(step_volume(&stub(0.4), -1.0).unwrap(), 0.0);
    }

    #[test]
    fn t_sysvol_03_rounds_to_one_percent() {
        let ep = stub(0.333);
        assert_eq!(step_volume(&ep, 0.0).unwrap(), 0.33);
        let ep = stub(0.3349);
        assert_eq!(step_volume(&ep, 0.0004).unwrap(), 0.34);
        // 설정값도 0.01 격자 위다
        assert_eq!(ep.level.get(), 0.34_f32);
    }

    #[test]
    fn t_sysvol_04_rejects_invalid_delta_without_touching_the_device() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1.01, -1.01, 5.0] {
            let ep = stub(0.5);
            let err = step_volume(&ep, bad).unwrap_err();
            assert_eq!(err.code(), ErrorCode::InvalidInput, "delta {bad}");
            assert_eq!(ep.sets.get(), 0);
            assert_eq!(ep.level.get(), 0.5);
        }
    }

    #[test]
    fn t_sysvol_05_device_failures_map_to_audio_device_unavailable() {
        let mut no_get = stub(0.5);
        no_get.fail_get = true;
        assert_eq!(
            step_volume(&no_get, 0.05).unwrap_err().code(),
            ErrorCode::AudioDeviceUnavailable
        );
        let mut no_set = stub(0.5);
        no_set.fail_set = true;
        let err = step_volume(&no_set, 0.05).unwrap_err();
        assert_eq!(err.code(), ErrorCode::AudioDeviceUnavailable);
        // 사용자 메시지는 고정 문구 - COM 상세를 싣지 않는다
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            serde_json::json!({"code": "audio_device_unavailable", "message": "audio device is not available"})
        );
    }

    // 분류: Validation - 명령 경계에서도 잘못된 delta 는 장치 접근 전에 거부된다 (실제 음량은 바꾸지 않는다)
    #[test]
    fn t_sysvol_06_command_rejects_invalid_delta_before_any_device_access() {
        let rt = tauri::async_runtime::block_on(system_volume_step(f64::NAN));
        assert_eq!(rt.unwrap_err().code(), ErrorCode::InvalidInput);
        let rt = tauri::async_runtime::block_on(system_volume_step(2.0));
        assert_eq!(rt.unwrap_err().code(), ErrorCode::InvalidInput);
    }
}
