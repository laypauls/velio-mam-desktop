//! UI 가 뜨기 전에 필요한 OS 수준 알림 — 시작 오류 대화상자와 기존 창 앞으로 가져오기 (P2-1, D6).
//!
//! 이 시점에는 WebView(i18n)가 없으므로 문구는 이 모듈의 상수로 둔다. 한국어 우선, 로케일 확장 시 함께 정리한다.

/// 표시 이름의 유일한 원천 — `concat!` 에서도 쓰도록 매크로로 둔다. 창 제목(`tauri.conf.json`)과 같아야 한다.
#[macro_export]
macro_rules! app_display_name {
    () => {
        "velio."
    };
}

/// 사용자에게 보이는 제품 이름. productName 은 폴더명 제약으로 마침표 없는 `velio` 이므로 별도 상수를 쓴다.
pub const APP_DISPLAY_NAME: &str = app_display_name!();
pub const ERROR_TITLE: &str = APP_DISPLAY_NAME;
/// `{log}` 자리에 로그 폴더 경로가 들어간다. 이름 뒤에 한국어 조사를 붙이지 않는다.
pub const STARTUP_FAILED: &str = concat!(
    app_display_name!(),
    " — 앱을 시작하지 못했습니다.\n\n자세한 내용은 로그 폴더를 확인하세요:\n{log}"
);
pub const LOCATION_FAILED: &str =
    "데이터 폴더를 준비하지 못했습니다.\n\n폴더 위치와 쓰기 권한을 확인하세요:\n{log}";

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// 오류 대화상자(모달). Windows 가 아니면 stderr 로만 알린다.
pub fn show_error(title: &str, message: &str) {
    eprintln!("{title}: {message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let (t, m) = (wide(title), wide(message));
        // SAFETY: 두 버퍼 모두 NUL 로 끝나고 호출 동안 살아 있다. 소유 창이 없으므로 null.
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                m.as_ptr(),
                t.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

/// 이미 실행 중인 인스턴스의 창을 복원하고 앞으로 가져온다(최선 노력 — 창이 아직 없으면 아무 일도 안 한다).
pub fn focus_existing_window(title: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        let t = wide(title);
        // SAFETY: `t` 는 NUL 로 끝나는 버퍼이고, 반환된 핸들은 즉시 쓰고 보관하지 않는다.
        unsafe {
            let hwnd = FindWindowW(std::ptr::null(), t.as_ptr());
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = title;
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Business rule (single-instance title match) / Boundary (no Korean particle after name)
    use super::*;

    #[test]
    fn t_brand_01_display_name_is_velio_with_period() {
        // Arrange / Act / Assert
        assert_eq!(APP_DISPLAY_NAME, "velio.");
        assert_eq!(ERROR_TITLE, APP_DISPLAY_NAME);
    }

    #[test]
    fn t_brand_02_main_window_title_in_tauri_conf_matches_display_name() {
        // Arrange: 단일 인스턴스 로직은 창 제목 완전 일치로 기존 창을 찾는다
        let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("read tauri.conf.json")
            .replace("\r\n", "\n");

        // Act
        let conf: serde_json::Value = serde_json::from_str(&raw).expect("parse tauri.conf.json");

        // Assert
        assert_eq!(
            conf["app"]["windows"][0]["title"].as_str(),
            Some(APP_DISPLAY_NAME)
        );
    }

    #[test]
    fn t_brand_03_startup_message_has_name_then_no_particle_and_log_placeholder() {
        // Arrange / Act
        let msg = STARTUP_FAILED;

        // Assert
        assert!(msg.starts_with("velio. — 앱을 시작하지 못했습니다."));
        assert!(msg.ends_with("{log}"));
    }
}
