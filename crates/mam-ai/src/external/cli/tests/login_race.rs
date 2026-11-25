//! 로그인 세션 경쟁·수명 시험 — 동시 시작(single-flight), URL 대기 중 취소, 세션 `Drop`, 전체 제한 시간.
//! 분류: Concurrency / Time / External / Business rule(프로세스를 남기지 않는다)

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::login::{Opener, URL, fx, login_script, wait_final};
use super::*;
use crate::external::cli::fake::Step;
use crate::external::cli::{CliLogin, LoginState, LoginTimings};

fn url_then_hang() -> Scripts {
    Scripts {
        login: vec![Step::Out(format!("{URL}\n")), Step::Hang],
        ..Default::default()
    }
}

fn login_spawns(f: &super::login::Fx) -> usize {
    f.runner.count_with_arg("--claudeai")
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn t_aicli_race_01_concurrent_starts_leave_exactly_one_live_session() {
    // Arrange
    let f = Arc::new(fx(url_then_hang(), FakeEnv::with_claude()));

    // Act: 두 시작이 동시에 들어온다
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let f = Arc::clone(&f);
            std::thread::spawn(move || f.login.start())
        })
        .collect();
    let results: Vec<LoginState> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Assert: 둘 다 정상 시작 -> 먼저 뜬 프로세스는 종료되고 하나만 산다 (고아 프로세스 없음)
    assert!(
        results
            .iter()
            .all(|r| matches!(r, LoginState::Waiting { .. })),
        "{results:?}"
    );
    assert_eq!(login_spawns(&f), 2);
    assert_eq!(f.runner.killed_pids().len(), 1);
    assert_eq!(f.host.exec().active_count(), 1);
    f.login.cancel();
    assert_eq!(f.host.exec().active_count(), 0);
}

#[test]
fn t_aicli_race_02_cancel_while_waiting_for_the_url_stops_the_new_process() {
    // Arrange: URL 이 나오지 않는 CLI + 긴 URL 대기
    let s = Scripts {
        login: vec![Step::Out("Starting login...\n".into()), Step::Hang],
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    let opener = Arc::new(Opener::default());
    let login = Arc::new(CliLogin::with_timings(
        Arc::clone(&f.host),
        Arc::clone(&opener) as _,
        LoginTimings {
            url_timeout: Duration::from_secs(10),
            ..LoginTimings::default()
        },
    ));
    let started = Instant::now();
    let t = {
        let login = Arc::clone(&login);
        std::thread::spawn(move || login.start())
    };
    wait_until("login spawn", || login_spawns(&f) == 1);

    // Act
    login.cancel();
    let result = t.join().unwrap();

    // Assert: 제한 시간(10초)을 기다리지 않고 끝나며 세션·프로세스·브라우저 호출이 남지 않는다
    assert_eq!(result, LoginState::None);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(f.runner.killed_pids().len(), 1);
    assert_eq!(f.host.exec().active_count(), 0);
    assert_eq!(login.status(), LoginState::None);
    assert!(opener.opened.lock().unwrap().is_empty());
}

#[test]
fn t_aicli_race_03_cancel_before_start_does_not_poison_the_next_start() {
    // 회귀 대조: 세대 번호는 시작 시점에 읽는다 - 이전 취소가 이후 시작을 막지 않는다
    let f = fx(url_then_hang(), FakeEnv::with_claude());
    f.login.cancel();

    let st = f.login.start();

    assert!(matches!(st, LoginState::Waiting { .. }), "{st:?}");
    f.login.cancel();
}

#[test]
fn t_aicli_race_04_dropping_the_login_kills_its_process() {
    // Arrange
    let f = fx(url_then_hang(), FakeEnv::with_claude());
    assert!(matches!(f.login.start(), LoginState::Waiting { .. }));
    assert_eq!(f.host.exec().active_count(), 1);
    let (runner, host, login) = (f.runner, f.host, f.login);

    // Act: 정리 없이 놓는다
    drop(login);

    // Assert
    assert_eq!(runner.killed_pids().len(), 1);
    assert_eq!(host.exec().active_count(), 0);
}

#[test]
fn t_aicli_race_05_session_timeout_kills_the_cli_and_reports_failed_unknown() {
    // Arrange: URL 은 받았지만 사용자가 승인하지 않는다
    let f = fx(url_then_hang(), FakeEnv::with_claude());
    let login = CliLogin::with_timings(
        Arc::clone(&f.host),
        Arc::new(Opener::default()),
        LoginTimings {
            url_timeout: Duration::from_millis(300),
            url_join: Duration::from_millis(30),
            session_timeout: Duration::from_millis(400),
        },
    );
    assert!(matches!(login.start(), LoginState::Waiting { .. }));

    // Act
    let end = wait_final(&login);

    // Assert
    assert_eq!(
        end,
        LoginState::Failed {
            error_code: "unknown".into()
        }
    );
    assert_eq!(f.runner.killed_pids().len(), 1);
    assert_eq!(f.host.exec().active_count(), 0);
    assert_eq!(login.status(), LoginState::None, "결과는 한 번만 전달된다");
}

#[test]
fn t_aicli_race_06_session_timeout_before_any_url_is_login_no_url() {
    // Arrange: URL 대기(5초)보다 세션 제한(200ms)이 먼저 온다
    let s = Scripts {
        login: vec![Step::Out("Starting login...\n".into()), Step::Hang],
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    let login = CliLogin::with_timings(
        Arc::clone(&f.host),
        Arc::new(Opener::default()),
        LoginTimings {
            url_timeout: Duration::from_secs(5),
            url_join: Duration::from_millis(30),
            session_timeout: Duration::from_millis(200),
        },
    );
    let started = Instant::now();

    let st = login.start();

    assert_eq!(
        st,
        LoginState::Failed {
            error_code: "login_no_url".into()
        }
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(f.host.exec().active_count(), 0);
}

#[test]
fn t_aicli_race_07_a_session_that_finishes_before_the_timeout_is_not_failed() {
    // 회귀 대조: 정상 완료는 제한 시간과 무관하다
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    assert!(matches!(f.login.start(), LoginState::Waiting { .. }));

    gate.store(true, std::sync::atomic::Ordering::SeqCst);

    assert_eq!(wait_final(&f.login), LoginState::Done { verified: true });
}
