//! `CliHost` 시험 — 탐지(설치·버전·로그인 상태)·apiKeyEnv·로그아웃(진행 중 요청 거부).
//! 분류: Happy / Boundary / External / Business rule

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::external::cli::{CliClient, LogoutError};
use crate::external::client::AiClient;
use crate::external::consent::ConsentGate;

fn mk(
    scripts: Scripts,
    env: FakeEnv,
) -> (
    tempfile::TempDir,
    Arc<crate::external::cli::fake::FakeRunner>,
    Arc<CliHost>,
) {
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(scripts);
    let h = host(Arc::clone(&runner), env, dir.path());
    (dir, runner, h)
}

#[test]
fn t_aicli_det_01_detects_version_account_and_never_sends_a_request() {
    let (_d, runner, h) = mk(Scripts::default(), FakeEnv::with_claude());

    let st = h.detect();

    assert!(st.found && st.supported);
    assert_eq!(st.version.as_deref(), Some("2.1.318"));
    let acc = st.account.unwrap();
    assert!(acc.logged_in);
    assert_eq!(acc.plan.as_deref(), Some("max"));
    assert!(!st.api_key_env);
    assert_eq!(
        runner.count_with_arg("-p"),
        0,
        "탐지는 모델 요청을 보내지 않는다"
    );
}

#[test]
fn t_aicli_det_02_not_installed_reports_found_false() {
    let (_d, runner, h) = mk(Scripts::default(), FakeEnv::without_claude());

    let st = h.detect();

    assert!(!st.found && !st.supported && st.account.is_none() && st.version.is_none());
    assert_eq!(runner.spawn_count(), 0);
}

#[test]
fn t_aicli_det_03_status_command_failure_is_unknown_not_logged_out() {
    for status in [
        vec![
            Step::Out("Usage: claude [options]\n".into()),
            Step::Exit(Some(1)),
        ],
        vec![Step::Exit(Some(2))],
    ] {
        let s = Scripts {
            status,
            ..Default::default()
        };
        let (_d, _r, h) = mk(s, FakeEnv::with_claude());

        let st = h.detect();

        assert!(st.found && st.supported);
        assert_eq!(
            st.account, None,
            "알 수 없음 — loggedIn=false 로 만들지 않는다"
        );
    }
}

#[test]
fn t_aicli_det_04_explicit_logged_out_account_is_reported() {
    let s = Scripts {
        status: vec![
            Step::Out(r#"{"loggedIn":false}"#.into()),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let (_d, _r, h) = mk(s, FakeEnv::with_claude());

    let acc = h.detect().account.unwrap();

    assert!(!acc.logged_in);
}

#[test]
fn t_aicli_det_05_unsupported_version_skips_the_status_command() {
    let s = Scripts {
        version: Some(vec![Step::Out("2.1.100\n".into()), Step::Exit(Some(0))]),
        ..Default::default()
    };
    let (_d, runner, h) = mk(s, FakeEnv::with_claude());

    let st = h.detect();

    assert!(st.found && !st.supported);
    assert_eq!(st.version.as_deref(), Some("2.1.100"));
    assert_eq!(st.account, None);
    assert_eq!(runner.count_with_arg("status"), 0);
}

#[test]
fn t_aicli_det_06_api_key_env_reports_presence_of_either_variable() {
    for (var, expect) in [
        ("ANTHROPIC_API_KEY", true),
        ("ANTHROPIC_AUTH_TOKEN", true),
        ("SOMETHING_ELSE", false),
    ] {
        let (_d, _r, h) = mk(Scripts::default(), FakeEnv::with_claude().with_var(var));
        assert_eq!(h.detect().api_key_env, expect, "{var}");
    }
}

#[test]
fn t_aicli_det_07_hung_version_probe_does_not_block_forever() {
    // 응답 없는 --version 은 PROBE_TIMEOUT(10s) 뒤 지원 안 함으로 처리된다. 시험에서는 종료 신호로 즉시 끊는다.
    let s = Scripts {
        version: Some(vec![Step::Hang]),
        ..Default::default()
    };
    let (_d, runner, h) = mk(s, FakeEnv::with_claude());
    let h2 = Arc::clone(&h);
    let t = std::thread::spawn(move || h2.detect());
    let deadline = Instant::now() + Duration::from_secs(5);
    while runner.spawn_count() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }

    h.kill_all();
    let st = t.join().unwrap();

    assert!(st.found && !st.supported);
}

#[test]
fn t_aicli_out_01_logout_runs_auth_logout() {
    let (_d, runner, h) = mk(Scripts::default(), FakeEnv::with_claude());

    assert_eq!(h.logout(), Ok(()));

    assert_eq!(runner.count_with_arg("logout"), 1);
}

#[test]
fn t_aicli_out_02_logout_failure_and_missing_cli() {
    let s = Scripts {
        logout: vec![Step::Exit(Some(1))],
        ..Default::default()
    };
    let (_d, _r, h) = mk(s, FakeEnv::with_claude());
    assert_eq!(h.logout(), Err(LogoutError::Failed));
    let (_d, runner, h) = mk(Scripts::default(), FakeEnv::without_claude());
    assert_eq!(h.logout(), Err(LogoutError::NotFound));
    assert_eq!(runner.spawn_count(), 0);
}

#[test]
fn t_aicli_out_03_logout_is_refused_while_a_request_is_running() {
    // Arrange: 멈춘 요청 하나
    let s = Scripts {
        request: vec![Step::Hang],
        ..Default::default()
    };
    let (_d, runner, h) = mk(s, FakeEnv::with_claude());
    let c = Arc::new(CliClient::new(Arc::clone(&h), None).with_cap(Duration::from_millis(600)));
    let t = {
        let c = Arc::clone(&c);
        std::thread::spawn(move || c.analyze(&ConsentGate::check(true).unwrap(), &request()))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while runner.count_with_arg("-p") == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }

    // Act
    let refused = h.logout();

    // Assert: 거부되고 로그아웃 명령은 실행되지 않았다. 요청이 끝나면 다시 가능하다.
    assert_eq!(refused, Err(LogoutError::Busy));
    assert_eq!(runner.count_with_arg("logout"), 0);
    let _ = t.join().unwrap();
    assert_eq!(h.logout(), Ok(()));
}
