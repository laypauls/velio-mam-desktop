//! 설치 보안 회귀 — 내보내기 확장자 허용 목록(M1), 표시 문자열의 서식 문자 제거(L4), 같은 ID 덮어쓰기 경고용 `has_kept_data`(L8),
//! 크기 상한 경계(L7). 실제 SQLite·임시 폴더.
//!
//! 분류: Validation / Business rule / Boundary(상한 ±1) / External(없는 데이터).
mod common_rt;

use common_rt::{EN, Rt};
use mam_plugin_runtime::{InstallChange, InstallError, ManifestError};

const ID: &str = "com.example.basic";

#[test]
fn t_pinst_sec_01_install_rejects_executable_export_extensions() {
    // Arrange
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.patch(&dir, "plugin.json", |s| s.replace("\"fileExtension\": \"csv\"", "\"fileExtension\": \"bat\""));

    // Act
    let inspect = rt.rt.inspect_dir(&dir, EN);
    let install = rt.rt.install(&dir, ID, "1.0.0", common_rt::basic_perms(), EN);

    // Assert: 점검도 설치도 거부되고 아무것도 설치되지 않는다
    assert!(matches!(inspect, Err(InstallError::Manifest(ManifestError::InvalidFileExtension { .. }))), "{inspect:?}");
    assert!(matches!(install, Err(InstallError::Manifest(ManifestError::InvalidFileExtension { .. }))));
    assert!(rt.rt.list(EN).is_empty());
}

#[test]
fn t_pinst_sec_02_display_strings_drop_bidi_override_and_control_chars() {
    // Arrange: 번역값에 RTL 재정의·제로폭·줄바꿈
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.patch(&dir, "l10n/en.json", |s| s.replace("\"Basic Plugin\"", "\"Safe\u{202E}gpj.exe\u{200B}\\u2028Name\"").replace("\"Echo\"", "\"Ec\u{202E}ho\""));

    // Act
    let preview = rt.rt.inspect_dir(&dir, EN).unwrap();
    rt.rt.install(&dir, ID, "1.0.0", common_rt::basic_perms(), EN).unwrap();
    let listed = rt.rt.list(EN);

    // Assert
    assert_eq!(preview.display_name, "Safegpj.exe Name");
    assert_eq!(listed[0].display_name, "Safegpj.exe Name");
    assert_eq!(preview.commands[0].title, "Echo");
    assert_eq!(listed[0].commands[0].title, "Echo");
}

#[test]
fn t_pinst_sec_03_preview_reports_kept_data_only_when_not_installed() {
    // Arrange: 처음엔 데이터 없음
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    let fresh = rt.rt.inspect_dir(&dir, EN).unwrap();
    assert_eq!((fresh.change, fresh.has_kept_data), (InstallChange::Install, false));

    // Act: 설치 + 데이터 + 보존 제거
    rt.install_basic(&dir);
    let asset = rt.add_asset("p/1.jpg");
    rt.seed_data(ID, 3, &[asset]);
    let installed = rt.rt.inspect_dir(&dir, EN).unwrap();
    rt.rt.uninstall(ID, true).unwrap();
    let kept = rt.rt.inspect_dir(&dir, EN).unwrap();

    // Assert: 설치돼 있으면 `installed` 가 경고 근거, 제거 후 보존 데이터는 `has_kept_data`
    assert_eq!((installed.change, installed.has_kept_data, installed.installed.is_some()), (InstallChange::Reinstall, false, true));
    assert_eq!((kept.change, kept.has_kept_data, kept.installed.is_some()), (InstallChange::Install, true, false));
}

#[test]
fn t_pinst_sec_04_main_one_byte_over_the_limit_is_rejected_and_at_the_limit_is_accepted() {
    // Arrange: 정확히 상한 크기의 main.js (끝에 주석으로 채운다)
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    let main = dir.join("dist").join("main.js");
    let limit = 2usize << 20;
    let head = format!("{}\n//", std::fs::read_to_string(&main).unwrap().trim_end());
    let sized = |n: usize| format!("{head}{}", "x".repeat(n - head.len()));

    // Act / Assert
    std::fs::write(&main, sized(limit)).unwrap();
    assert!(rt.rt.inspect_dir(&dir, EN).is_ok(), "exactly at the limit");
    std::fs::write(&main, sized(limit + 1)).unwrap();
    assert!(matches!(rt.rt.inspect_dir(&dir, EN), Err(InstallError::MainTooLarge)));
}
