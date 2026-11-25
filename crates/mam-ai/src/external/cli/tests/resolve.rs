//! 실행 파일 탐색 시험 — PATH `.exe`, npm `.cmd` 래퍼 -> `.exe`, 공식 설치 위치, 미설치, 비 Windows.
//! 분류: Happy / Boundary / Validation

use std::ffi::OsString;
use std::path::PathBuf;

use super::*;
use crate::external::cli::resolve_claude;

fn env(path: &str) -> FakeEnv {
    FakeEnv {
        path: Some(OsString::from(path)),
        windows: true,
        ..Default::default()
    }
}

#[test]
fn t_aicli_res_01_prefers_exe_on_path() {
    let e = env(r"C:\a;C:\b");
    e.add_file(r"C:\b\claude.exe", "");
    e.add_file(r"C:\a\claude.cmd", "");
    assert_eq!(resolve_claude(&e), Some(PathBuf::from(r"C:\b\claude.exe")));
}

#[test]
fn t_aicli_res_02_follows_npm_cmd_shim_to_the_exe_without_a_shell() {
    let e = env(r"C:\npm");
    e.add_file(
        r"C:\npm\claude.cmd",
        "@ECHO off\r\nendlocal & goto #_undefined_# 2>NUL || \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n",
    );
    e.add_file(
        r"C:\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe",
        "",
    );

    let r = resolve_claude(&e).unwrap();

    assert_eq!(
        r,
        PathBuf::from(r"C:\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe")
    );
}

#[test]
fn t_aicli_res_03_shim_pointing_at_a_script_or_missing_exe_is_not_resolved() {
    let e = env(r"C:\npm");
    e.add_file(r"C:\npm\claude.cmd", "node \"%dp0%\\cli.js\" %*");
    assert_eq!(
        resolve_claude(&e),
        None,
        ".js 를 가리키는 래퍼는 직접 실행할 수 없다"
    );
    let e = env(r"C:\npm");
    e.add_file(r"C:\npm\claude.cmd", "\"%dp0%\\bin\\claude.exe\" %*");
    assert_eq!(resolve_claude(&e), None, "대상 .exe 가 없다");
}

#[test]
fn t_aicli_res_04_falls_back_to_the_official_install_location() {
    let mut e = env(r"C:\nothing");
    e.home = Some(PathBuf::from(r"C:\Users\me"));
    e.add_file(r"C:\Users\me\.local\bin\claude.exe", "");
    assert_eq!(
        resolve_claude(&e),
        Some(PathBuf::from(r"C:\Users\me\.local\bin\claude.exe"))
    );
}

#[test]
fn t_aicli_res_05_not_installed_and_empty_path_return_none() {
    assert_eq!(resolve_claude(&env(r"C:\nothing")), None);
    let mut e = env("");
    e.path = None;
    assert_eq!(resolve_claude(&e), None);
}
