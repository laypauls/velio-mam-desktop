//! 시스템 폴더 자동 제외 통합 테스트 (T-sysdir-01 ~ 05) — 스캔(`list_dir`)과 감시 이벤트(`apply_path`)가 같은 규칙.
//!
//! 분류: Business rule (01~04 제외 규칙) / Boundary (03 일반 폴더 회귀, 이름 대소문자) / Validation (원본 불변 단언).
#![cfg(windows)]

mod common;

use std::fs;
use std::process::Command;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::scan::{PathAction, apply_path, scan_folder};

fn register_and_scan(env: &Env) -> i64 {
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    id
}

fn active_paths(env: &Env) -> Vec<String> {
    env.read(|c| {
        let mut stmt = c
            .prepare("SELECT rel_path FROM file_locations WHERE detached_at IS NULL ORDER BY rel_path")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|p| {
                let p = p.unwrap();
                p.strip_prefix("lib\\").map_or_else(|| p.clone(), str::to_owned)
            })
            .collect()
    })
}

/// 디렉터리에 속성을 건다 (`attrib` — 테스트 픽스처 전용, 임시 디렉터리 안).
fn attrib(dir: &std::path::Path, flags: &[&str]) {
    let status = Command::new("attrib")
        .args(flags)
        .arg(mam_core::volume::strip_verbatim(&dir.to_string_lossy())) // attrib 은 verbatim 접두를 못 읽는다
        .status()
        .expect("attrib runs");
    assert!(status.success(), "attrib {flags:?} failed");
}

#[test]
fn t_sysdir_01_scan_skips_reserved_volume_folders_case_insensitively() {
    // Arrange
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    env.write("lib/$RECYCLE.BIN/S-1-5/trash.jpg", 2);
    env.write("lib/System Volume Information/idx.jpg", 3);
    env.write("lib/$Extend/ext.jpg", 4);
    env.write("lib/$extend2/ok.jpg", 5); // 이름이 비슷할 뿐인 일반 폴더는 대상
    env.write("lib/sub/$recycle.bin/low.jpg", 6); // 하위의 소문자 표기도 제외
    let before = snapshot(&env.vol);

    // Act
    register_and_scan(&env);

    // Assert
    assert_eq!(active_paths(&env), [r"$extend2\ok.jpg", "keep.jpg"]);
    assert_eq!(snapshot(&env.vol), before, "originals untouched");
}

#[test]
fn t_sysdir_02_scan_skips_directories_with_hidden_and_system_attributes() {
    // Arrange
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    env.write("lib/both/a.jpg", 2);
    env.write("lib/plain/b.jpg", 3);
    attrib(&env.path("lib/both"), &["+h", "+s"]);
    let before = snapshot(&env.vol);

    // Act
    register_and_scan(&env);

    // Assert: 속성이 붙은 디렉터리만 제외, 일반 폴더는 그대로
    assert_eq!(active_paths(&env), ["keep.jpg", r"plain\b.jpg"]);
    assert_eq!(snapshot(&env.vol), before);
}

#[test]
fn t_sysdir_03_watch_events_under_system_folders_are_ignored_and_normal_ones_apply() {
    // Arrange
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    let id = register_and_scan(&env);
    let recycle = env.write("lib/$RECYCLE.BIN/new.jpg", 2);
    let svi = env.write("lib/system volume information/new.jpg", 3);
    let ext = env.write("lib/$EXTEND/new.jpg", 4);
    let hs = env.write("lib/hs/new.jpg", 5);
    attrib(&env.path("lib/hs"), &["+h", "+s"]);
    let normal = env.write("lib/normal/new.jpg", 6);

    // Act
    let actions: Vec<_> = [&recycle, &svi, &ext, &hs, &normal]
        .iter()
        .map(|p| apply_path(env.ingest.env(), id, p).unwrap())
        .collect();

    // Assert: 시스템 폴더 4건은 무시, 일반 폴더만 반영
    assert_eq!(&actions[..4], &[PathAction::Ignored; 4]);
    assert_eq!(actions[4], PathAction::Reconciled);
    assert_eq!(active_paths(&env), ["keep.jpg", r"normal\new.jpg"]);
}

#[test]
fn t_sysdir_04_watch_event_for_a_hidden_system_directory_itself_is_ignored() {
    // Arrange
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    let id = register_and_scan(&env);
    env.write("lib/hs/x.jpg", 2);
    attrib(&env.path("lib/hs"), &["+h", "+s"]);

    // Act: 디렉터리 자신에 대한 이벤트
    let action = apply_path(env.ingest.env(), id, &env.path("lib/hs")).unwrap();

    // Assert
    assert_eq!(action, PathAction::Ignored);
    assert_eq!(active_paths(&env), ["keep.jpg"]);
}

#[test]
fn t_sysdir_05_registered_root_with_hidden_system_attributes_is_still_scanned() {
    // Arrange: 볼륨 루트처럼 루트 자신이 숨김+시스템 속성인 경우 (자식만 검사한다)
    let env = Env::new();
    env.write("lib/keep.jpg", 1);
    attrib(&env.path("lib"), &["+h", "+s"]);
    fs::write(env.path("lib/second.jpg"), [7u8; 70]).unwrap();

    // Act
    let id = register_and_scan(&env);
    let action = apply_path(env.ingest.env(), id, &env.path("lib/second.jpg")).unwrap();

    // Assert
    assert_eq!(action, PathAction::Reconciled);
    assert_eq!(active_paths(&env), ["keep.jpg", "second.jpg"]);
    attrib(&env.path("lib"), &["-h", "-s"]); // 임시 디렉터리 정리가 막히지 않게
}
