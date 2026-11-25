//! 스캔 멈춤 감시 (T-stall-01 ~ 07) — 3분 넘게 같은 경로를 붙들면 폴더 상태 `scanStall`, 로그는 10분마다·풀리면 1회.
//!
//! 분류: Boundary (01 2:59 vs 3:00) / Business rule (02 경로가 바뀌면 해제, 03 처리 진행이 있으면 해제, 04 같은 경로는 유지,
//! 06 스캔 종료 = 해제, 07 폴더별 독립) / Time (05 로그 주기, 시계 주입) / 계약 (08 JSON 모양, 정상이면 키 없음).
//! 실제 SQLite + 임시 폴더. 진행 상태(`ScanProgressCell`)를 직접 조작해 시계를 결정적으로 만든다.
#![cfg(windows)]

mod common;

use common::env::Env;
use mam_core::ingest::{STALL_LOG_EVERY_MS, StallEvent};
use mam_core::scan::STALL_AFTER_MS;

const THREE_MIN: i64 = 180_000;

/// 폴더 하나를 등록하고 스캔이 시작된 것처럼 진행 상태를 연다.
fn scanning(env: &Env, rel: &str) -> i64 {
    env.write(&format!("{rel}/x.jpg"), 1);
    let id = env.ingest.add_folder(env.path(rel).to_str().unwrap()).unwrap().folder_id;
    env.ingest
        .env()
        .progress
        .start(id, format!(r"X:\{rel}"), 0, env.clock_now());
    id
}

fn stall_of(env: &Env, id: i64) -> Option<mam_core::scan::ScanStall> {
    env.ingest.list_folders().unwrap().into_iter().find(|f| f.folder_id == id).unwrap().scan_stall.clone()
}

#[test]
fn t_stall_01_stalled_exactly_at_three_minutes_not_one_millisecond_before() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    let since = env.clock_now();
    assert_eq!(STALL_AFTER_MS, THREE_MIN);

    // Act / Assert: 2분 59.999초
    env.clock.advance(THREE_MIN - 1);
    assert!(env.ingest.scan_stalls().is_empty());
    assert_eq!(stall_of(&env, id), None);

    // 정확히 3분
    env.clock.advance(1);
    let stall = stall_of(&env, id).expect("stalled at 3:00");
    assert_eq!(stall.at_path, r"X:\lib");
    assert_eq!(stall.since, since);
}

#[test]
fn t_stall_02_moving_to_another_path_clears_the_stall_and_logs_one_recovery() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    env.clock.advance(THREE_MIN);
    assert!(matches!(env.ingest.check_stalls().as_slice(), [StallEvent::Stalled { folder_id, .. }] if *folder_id == id));

    // Act: 다른 경로로 넘어갔다
    env.ingest.env().progress.hold(id, r"X:\lib\next", env.clock_now());

    // Assert
    assert_eq!(stall_of(&env, id), None);
    assert_eq!(env.ingest.check_stalls(), vec![StallEvent::Recovered { folder_id: id }]);
    assert!(env.ingest.check_stalls().is_empty(), "recovery is logged once");
}

#[test]
fn t_stall_03_processing_progress_restarts_the_clock() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    env.clock.advance(THREE_MIN + 5_000);
    assert!(stall_of(&env, id).is_some());

    // Act: 같은 경로라도 건수가 늘면 진행이다
    env.ingest.env().progress.add(id, 10, env.clock_now());

    // Assert: 시계가 다시 시작 — 2:59.999 까지는 멈춤이 아니다
    assert_eq!(stall_of(&env, id), None);
    env.clock.advance(THREE_MIN - 1);
    assert_eq!(stall_of(&env, id), None);
    env.clock.advance(1);
    assert!(stall_of(&env, id).is_some());
}

#[test]
fn t_stall_04_holding_the_same_path_again_does_not_restart_the_clock() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    let since = env.clock_now();

    // Act: 같은 경로를 반복해서 기록한다 (파일 하나가 계속 막혀 있다)
    for _ in 0..3 {
        env.clock.advance(THREE_MIN / 3);
        env.ingest.env().progress.hold(id, r"X:\lib", env.clock_now());
    }

    // Assert
    assert_eq!(stall_of(&env, id).map(|s| s.since), Some(since));
}

#[test]
fn t_stall_05_stall_is_logged_every_ten_minutes_with_an_injected_clock() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    env.clock.advance(THREE_MIN);

    // Act / Assert: 시작 1회 → 10분 안에는 조용 → 10분에 다시 1회
    assert_eq!(env.ingest.check_stalls().len(), 1);
    env.clock.advance(STALL_LOG_EVERY_MS - 1);
    assert!(env.ingest.check_stalls().is_empty());
    env.clock.advance(1);
    let again = env.ingest.check_stalls();
    assert!(matches!(again.as_slice(), [StallEvent::Stalled { folder_id, .. }] if *folder_id == id), "{again:?}");
}

#[test]
fn t_stall_06_finishing_the_scan_clears_the_stall() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    env.clock.advance(THREE_MIN);
    assert_eq!(env.ingest.check_stalls().len(), 1);

    // Act
    env.ingest.env().progress.clear(id);

    // Assert
    assert_eq!(stall_of(&env, id), None);
    assert_eq!(env.ingest.check_stalls(), vec![StallEvent::Recovered { folder_id: id }]);
}

#[test]
fn t_stall_07_stalls_are_tracked_per_folder() {
    // Arrange: 폴더 둘 — 하나만 오래 붙들고 있다
    let env = Env::new();
    let stuck = scanning(&env, "lib1");
    env.clock.advance(THREE_MIN - 1_000);
    let moving = scanning(&env, "lib2");

    // Act
    env.clock.advance(1_000);

    // Assert
    assert!(stall_of(&env, stuck).is_some());
    assert_eq!(stall_of(&env, moving), None);
}

#[test]
fn t_stall_08_json_omits_the_key_when_healthy_and_uses_camel_case_when_stalled() {
    // Arrange
    let env = Env::new();
    let id = scanning(&env, "lib");
    let json = |env: &Env| {
        let v = env.ingest.list_folders().unwrap().into_iter().find(|f| f.folder_id == id).unwrap();
        serde_json::to_value(v).unwrap()
    };

    // Act / Assert
    assert!(json(&env).get("scanStall").is_none(), "optional field is absent while healthy");
    env.clock.advance(THREE_MIN);
    let v = json(&env);
    assert_eq!(v["scanStall"]["atPath"], r"X:\lib");
    assert_eq!(v["scanStall"]["since"], env.clock_now() - THREE_MIN);
}
