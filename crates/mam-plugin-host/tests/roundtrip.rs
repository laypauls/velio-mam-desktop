//! 정상 경로·오류 전달 시험. 분류: Happy / Validation(미등록·async) / Business rule(플러그인 id 는 호스트가 정함)
mod common;

use common::*;
use mam_plugin_host::{HostState, InvokeError, InvokeKind, InvokeScope, LogLevel};
use serde_json::json;

#[test]
fn t_phost_rt_01_round_trip_with_host_call_and_provider() {
    // Arrange
    let (host, _, _) = new_host(PLUGIN, |_| {});
    // Act
    let cmd = run(&host, "hostcall", json!({"n": 41})).unwrap();
    let prov = host.invoke(InvokeKind::Provider, "meta.extract", json!({"asset": 7}), InvokeScope::empty()).unwrap();
    // Assert
    assert_eq!(cmd, json!({"got": {"n": 41}}));
    assert_eq!(prov, json!({"kind": "provider", "p": {"asset": 7}}));
    assert_eq!(host.starts(), 1, "두 호출이 같은 워커를 쓴다");
    assert!(matches!(host.state(), HostState::Running { .. }));
}

#[test]
fn t_phost_rt_02_plugin_id_comes_from_the_host_not_the_worker() {
    let (host, api, _) = new_host(PLUGIN, |_| {});
    run(&host, "hostcall", json!({"n": 1})).unwrap();
    assert_eq!(api.calls.lock().unwrap().as_slice(), &[("com.example.test".to_owned(), "echo".to_owned())]);
}

#[test]
fn t_phost_rt_03_host_error_reaches_js_as_exception_with_code() {
    let (host, _, _) = new_host(PLUGIN, |_| {});
    assert_eq!(run(&host, "hosterr", json!(null)).unwrap(), json!({"code": "denied", "message": "nope"}));
    // 처리되지 않은 호스트 오류는 스크립트 오류로 돌아오고 워커는 산다 (결함 아님)
    let e = run(&host, "unknown_host", json!(null)).unwrap_err();
    assert!(matches!(&e, InvokeError::Script(m) if m.contains("unknown host method")), "{e:?}");
    assert!(!e.is_fault());
    ok_echo(&host);
    assert_eq!(host.starts(), 1);
}

#[test]
fn t_phost_rt_04_script_exception_is_an_error_not_a_fault() {
    let (host, _, _) = new_host(PLUGIN, |_| {});
    let e = run(&host, "throw", json!(null)).unwrap_err();
    assert!(matches!(&e, InvokeError::Script(m) if m == "boom"), "{e:?}");
    ok_echo(&host);
    assert_eq!(host.starts(), 1, "예외로 워커를 버리지 않는다");
}

#[test]
fn t_phost_rt_05_unregistered_handler_and_async_handler_are_rejected() {
    let (host, _, _) = new_host(PLUGIN, |_| {});
    assert_eq!(run(&host, "nope", json!(null)).unwrap_err(), InvokeError::NotRegistered("nope".into()));
    // 종류가 다르면(명령 id 를 provider 로) 미등록
    assert!(matches!(host.invoke(InvokeKind::Provider, "echo", json!(null), InvokeScope::empty()), Err(InvokeError::NotRegistered(_))));
    let e = run(&host, "async", json!(null)).unwrap_err();
    assert!(matches!(&e, InvokeError::Script(m) if m.contains("async")), "{e:?}");
}

#[test]
fn t_phost_rt_06_logs_reach_host_truncated_on_char_boundary() {
    let (host, api, _) = new_host(PLUGIN, |_| {});
    run(&host, "logs", json!(null)).unwrap();
    let logs = api.logs.lock().unwrap();
    assert_eq!(logs[0], (LogLevel::Warn, r#"w {"a":1}"#.to_owned()));
    assert_eq!(logs[1].0, LogLevel::Error);
    assert!(logs[1].1.len() <= 4096 && logs[1].1.chars().all(|c| c == '가'));
}

#[test]
fn t_phost_rt_07_result_over_line_limit_becomes_error_and_worker_survives() {
    // Arrange: 줄 상한 1 MiB, 결과 2 MiB
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.max_line_bytes = 1 << 20);
    // Act
    let e = run(&host, "bigresult", json!(null)).unwrap_err();
    // Assert: 워커가 스스로 too_large 오류로 바꾼다 — 호스트가 죽이지 않는다
    assert!(matches!(&e, InvokeError::Plugin { code, .. } if code == "too_large"), "{e:?}");
    ok_echo(&host);
    assert_eq!(host.starts(), 1);
}

#[test]
fn t_phost_rt_08_init_failures_are_reported_and_counted_as_faults() {
    // 문법 오류·로드 중 예외·중복 등록
    for src in ["this is not js (", "throw new Error('load failed');", "mam.commands.register('a', () => 1); mam.commands.register('a', () => 2);"] {
        let (host, _, _) = new_host(src, |_| {});
        let e = run(&host, "a", json!(null)).unwrap_err();
        assert!(matches!(e, InvokeError::Init(_)), "{src}: {e:?}");
        assert_eq!(host.state(), HostState::Idle);
    }
}

// 분류: Business rule(범위·마감은 호출자·감독자가 정한다) / Validation(범위 상한)

#[test]
fn t_phost_scope_01_scope_is_sorted_deduped_and_deadline_stamped_by_supervisor() {
    // Arrange: 호출자가 만든 범위는 마감이 의미 없는 값(과거)이다
    let (host, api, _) = new_host(PLUGIN, |_| {});
    let scope = InvokeScope::new(vec![9, 3, 3, 5]).unwrap().with_deadline(std::time::Instant::now());
    let before = std::time::Instant::now();
    // Act
    host.invoke_with_timeout(InvokeKind::Command, "hostcall", json!({"n": 1}), scope, std::time::Duration::from_secs(7)).unwrap();
    // Assert
    let seen = api.scopes.lock().unwrap();
    assert_eq!(seen[0].0, vec![3, 5, 9]);
    let remain = seen[0].1.duration_since(before);
    assert!(remain >= std::time::Duration::from_secs(7) && remain < std::time::Duration::from_secs(9), "{remain:?}");
}

#[test]
fn t_phost_scope_02_scope_limits_and_contains() {
    let s = InvokeScope::new((1..=10_000).collect()).unwrap();
    assert!(s.contains(1) && s.contains(10_000) && !s.contains(0) && !s.contains(10_001));
    assert!(InvokeScope::new((1..=10_001).collect()).is_err());
    assert!(!InvokeScope::empty().contains(1));
    // 중복은 상한에 세지 않는다
    assert!(InvokeScope::new(vec![1; 20_000]).is_ok());
}
