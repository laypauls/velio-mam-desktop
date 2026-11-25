//! libmpv C API 의 최소 부분집합 — `libloading` 으로 심볼 해석 (빌드 시 링크하지 않는다).
//! 시그니처 근거: `temp/poc-mpv/lib/include/mpv/{client.h,render.h}` (mpv v0.42.0).

use libloading::Library;
use std::ffi::{CStr, CString, c_char, c_int, c_ulong, c_void};
use std::path::Path;
use std::sync::Arc;

pub type Ptr = *mut c_void;

// mpv_format
pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;

// mpv_event_id
pub const EV_SHUTDOWN: c_int = 1;
pub const EV_LOG_MESSAGE: c_int = 2;
pub const EV_END_FILE: c_int = 7;
pub const EV_FILE_LOADED: c_int = 8;
pub const EV_PLAYBACK_RESTART: c_int = 21;
pub const EV_PROPERTY_CHANGE: c_int = 22;

// mpv_end_file_reason
pub const END_FILE_ERROR: c_int = 4;

// mpv_render_param_type
pub const RP_API_TYPE: c_int = 1;
pub const RP_SKIP_RENDERING: c_int = 13;
pub const RP_SW_SIZE: c_int = 17;
pub const RP_SW_FORMAT: c_int = 18;
pub const RP_SW_STRIDE: c_int = 19;
pub const RP_SW_POINTER: c_int = 20;
pub const RP_D3D11_INIT_PARAMS: c_int = 21;
pub const RP_D3D11_FBO: c_int = 22;

/// mpv_render_update_flag
pub const UPDATE_FRAME: u64 = 1;

#[repr(C)]
pub struct MpvEvent {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventProperty {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct EventEndFile {
    pub reason: c_int,
    pub error: c_int,
}

#[repr(C)]
pub struct EventLog {
    pub prefix: *const c_char,
    pub level: *const c_char,
    pub text: *const c_char,
}

#[repr(C)]
pub struct RenderParam {
    pub typ: c_int,
    pub data: *mut c_void,
}

pub type UpdateFn = unsafe extern "C" fn(*mut c_void);

macro_rules! api {
    ($( $name:ident : $ty:ty ),* $(,)?) => {
        /// 해석된 함수 포인터. `Library` 는 마지막에 drop 되도록 뒤에 둔다
        pub struct Api {
            $( pub $name: $ty, )*
            _lib: Library,
        }

        impl Api {
            fn load(lib: Library) -> Result<Self, String> {
                Ok(Api {
                    $( $name: unsafe {
                        let sym: libloading::Symbol<$ty> = lib
                            .get(concat!("mpv_", stringify!($name), "\0").as_bytes())
                            .map_err(|_| format!("libmpv 심볼 없음: mpv_{}", stringify!($name)))?;
                        *sym
                    }, )*
                    _lib: lib,
                })
            }
        }
    };
}

api! {
    client_api_version: unsafe extern "C" fn() -> c_ulong,
    create: unsafe extern "C" fn() -> Ptr,
    initialize: unsafe extern "C" fn(Ptr) -> c_int,
    terminate_destroy: unsafe extern "C" fn(Ptr),
    set_option_string: unsafe extern "C" fn(Ptr, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(Ptr, *const c_char, *const c_char) -> c_int,
    set_property: unsafe extern "C" fn(Ptr, *const c_char, c_int, *mut c_void) -> c_int,
    get_property: unsafe extern "C" fn(Ptr, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(Ptr, *const *const c_char) -> c_int,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    observe_property: unsafe extern "C" fn(Ptr, u64, *const c_char, c_int) -> c_int,
    wait_event: unsafe extern "C" fn(Ptr, f64) -> *mut MpvEvent,
    wakeup: unsafe extern "C" fn(Ptr),
    free: unsafe extern "C" fn(*mut c_void),
    request_log_messages: unsafe extern "C" fn(Ptr, *const c_char) -> c_int,
    render_context_create: unsafe extern "C" fn(*mut Ptr, Ptr, *mut RenderParam) -> c_int,
    render_context_set_update_callback: unsafe extern "C" fn(Ptr, Option<UpdateFn>, *mut c_void),
    render_context_update: unsafe extern "C" fn(Ptr) -> u64,
    render_context_render: unsafe extern "C" fn(Ptr, *mut RenderParam) -> c_int,
    render_context_report_swap: unsafe extern "C" fn(Ptr),
    render_context_free: unsafe extern "C" fn(Ptr),
}

/// 포인터를 스레드 간에 옮기기 위한 래퍼 (mpv_handle·render context 는 문서상 지정한 규칙 아래 스레드 간 사용 가능)
#[derive(Clone, Copy)]
pub struct SendPtr(pub Ptr);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

/// 로드된 라이브러리 + mpv 핸들. 모든 mpv_* 호출은 여기를 거친다
pub struct Mpv {
    pub api: Arc<Api>,
    pub handle: SendPtr,
}

/// `dir` 아래 `libmpv-2.dll` 을 로드한다
pub fn load_api(dir: &Path) -> Result<Arc<Api>, String> {
    let dll = dir.join("libmpv-2.dll");
    let lib = unsafe { Library::new(&dll) }.map_err(|e| format!("libmpv 로드 실패 {}: {e}", dll.display()))?;
    Ok(Arc::new(Api::load(lib)?))
}

fn cstr(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| format!("NUL 포함 문자열: {s:?}"))
}

impl Mpv {
    pub fn err_text(&self, code: c_int) -> String {
        let p = unsafe { (self.api.error_string)(code) };
        if p.is_null() { format!("mpv error {code}") } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
    }

    fn check(&self, what: &str, code: c_int) -> Result<(), String> {
        if code < 0 { Err(format!("{what}: {}", self.err_text(code))) } else { Ok(()) }
    }

    /// 초기화 전 옵션
    pub fn set_option(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        self.check(&format!("option {name}"), unsafe { (self.api.set_option_string)(self.handle.0, n.as_ptr(), v.as_ptr()) })
    }

    pub fn set_prop(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (cstr(name)?, cstr(value)?);
        self.check(&format!("property {name}"), unsafe { (self.api.set_property_string)(self.handle.0, n.as_ptr(), v.as_ptr()) })
    }

    pub fn set_prop_double(&self, name: &str, mut value: f64) -> Result<(), String> {
        let n = cstr(name)?;
        self.check(&format!("property {name}"), unsafe { (self.api.set_property)(self.handle.0, n.as_ptr(), FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void) })
    }

    pub fn get_double(&self, name: &str) -> Option<f64> {
        let n = cstr(name).ok()?;
        let mut v = 0f64;
        let r = unsafe { (self.api.get_property)(self.handle.0, n.as_ptr(), FORMAT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_int(&self, name: &str) -> Option<i64> {
        let n = cstr(name).ok()?;
        let mut v = 0i64;
        let r = unsafe { (self.api.get_property)(self.handle.0, n.as_ptr(), FORMAT_INT64, &mut v as *mut i64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let n = cstr(name).ok()?;
        let mut p: *mut c_char = std::ptr::null_mut();
        let r = unsafe { (self.api.get_property)(self.handle.0, n.as_ptr(), FORMAT_STRING, &mut p as *mut *mut c_char as *mut c_void) };
        if r < 0 || p.is_null() {
            return None;
        }
        let s = unsafe { lossy(p) };
        unsafe { (self.api.free)(p as *mut c_void) };
        Some(s)
    }

    pub fn get_flag(&self, name: &str) -> Option<bool> {
        let n = cstr(name).ok()?;
        let mut v: c_int = 0;
        let r = unsafe { (self.api.get_property)(self.handle.0, n.as_ptr(), FORMAT_FLAG, &mut v as *mut c_int as *mut c_void) };
        (r >= 0).then_some(v != 0)
    }

    /// `mpv_command` (인자 배열 형태 — 경로의 공백·따옴표를 파싱하지 않는다)
    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        self.check(&format!("command {}", args.first().copied().unwrap_or("")), unsafe { (self.api.command)(self.handle.0, ptrs.as_ptr()) })
    }

    pub fn observe(&self, id: u64, name: &str, format: c_int) -> Result<(), String> {
        let n = cstr(name)?;
        self.check(&format!("observe {name}"), unsafe { (self.api.observe_property)(self.handle.0, id, n.as_ptr(), format) })
    }
}

/// C 문자열 → String (NULL 이면 빈 문자열)
pub unsafe fn lossy(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}
