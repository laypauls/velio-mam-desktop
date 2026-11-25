//! 분류: Validation / Business rule(내보내기 확장자 허용 목록). 순수 crate 라 External·Concurrency 해당 없음.

use mam_plugin_manifest::{
    EXPORT_EXTENSIONS, ManifestError as E, export_extension_allowed, parse_manifest,
};
use serde_json::json;

fn parse_ext(ext: &str) -> Result<(), E> {
    let v = json!({
        "apiVersion": 1, "id": "com.example.min", "version": "0.1.0",
        "engines": { "mam": ">=0.1.0" }, "main": "main.js", "displayName": "plugin.name",
        "permissions": ["export.write"],
        "contributes": { "providers": { "export": [{ "id": "p", "title": "t", "fileExtension": ext }] } }
    });
    parse_manifest(&v.to_string()).map(|_| ())
}

#[test]
fn t_pmanifest_ext_01_allowlist_accepts_data_formats() {
    assert!(!EXPORT_EXTENSIONS.is_empty());
    for ext in EXPORT_EXTENSIONS {
        assert!(parse_ext(ext).is_ok(), "{ext}");
    }
}

#[test]
fn t_pmanifest_ext_02_executable_and_script_formats_rejected() {
    for ext in [
        "exe", "bat", "cmd", "ps1", "hta", "lnk", "url", "scf", "vbs", "js", "msi", "scr", "cpl",
        "reg", "html", "htm", "svg", "dll", "com", "jar", "wsf", "CSV", "csv ", ".csv", "",
    ] {
        assert!(
            matches!(parse_ext(ext), Err(E::InvalidFileExtension { .. })),
            "{ext:?}"
        );
        assert!(!export_extension_allowed(ext), "{ext:?}");
    }
}

#[test]
fn t_pmanifest_ext_03_error_display_neutralizes_control_and_bidi_chars() {
    // 오류 메시지는 UI·로그로 나간다 — 줄바꿈·방향 재정의가 그대로 실리면 안 된다
    let err = parse_ext("a\nFAKE LOG LINE\u{202E}x")
        .unwrap_err()
        .to_string();
    assert!(!err.contains('\n') && !err.contains('\u{202E}'), "{err:?}");
}
