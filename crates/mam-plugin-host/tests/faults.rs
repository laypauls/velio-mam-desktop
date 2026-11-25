//! 크래시·차단기·프로토콜 위반 시험. 분류: External(프로세스 종료) / Time(가짜 시계) / Validation(깨진·거대 줄) / Business rule(10분 3회 비활성)
mod common;

use std::time::Duration;

use common::*;
use mam_plugin_host::{HostConfig, HostState, InvokeError, PluginHost};
use serde_json::json;
use std::sync::Arc;

/// 사전 호출로 워커를 띄우고 PID 를 가짜 API 에 알려 둔 뒤 `kill_me` 로 호출 도중 PID 종료시킨다
fn crash_once(host: &PluginHost, api: &FakeApi) -> InvokeError {
    ok_echo(host);
    let HostState::Running { pid } = host.state() else { panic!("running") };
    *api.kill_pid.lock().unwrap() = Some(pid);
    run(host, "kill_me", json!(null)).unwrap_err()
}

#[test]
fn t_phost_flt_01_crash_mid_call_errors_and_next_call_restarts() {
    let (host, api, _) = new_host(PLUGIN, |_| {});
    let e = crash_once(&host, &api);
    assert_eq!(e, InvokeError::Crashed);
    ok_echo(&host);
    assert_eq!(host.starts(), 2);
}

#[test]
fn t_phost_flt_02_three_crashes_within_ten_minutes_disable_the_plugin() {
    // Arrange
    let (host, api, clock) = new_host(PLUGIN, |_| {});
    // Act: 3 번 크래시 (가짜 시계 — 매번 2 분 간격)
    for _ in 0..3 {
        assert_eq!(crash_once(&host, &api), InvokeError::Crashed);
        clock.advance(Duration::from_secs(120));
    }
    // Assert: 비활성 — 새 워커를 띄우지 않는다
    let starts = host.starts();
    assert_eq!(host.state(), HostState::Disabled);
    assert_eq!(run(&host, "echo", json!(1)).unwrap_err(), InvokeError::Disabled);
    assert_eq!(host.starts(), starts);
    // 사용자가 다시 켜면 동작
    host.reenable();
    ok_echo(&host);
}

#[test]
fn t_phost_flt_03_crashes_spread_beyond_the_window_do_not_disable() {
    let (host, api, clock) = new_host(PLUGIN, |_| {});
    for _ in 0..4 {
        assert_eq!(crash_once(&host, &api), InvokeError::Crashed);
        clock.advance(Duration::from_secs(601)); // 크래시 간격이 항상 창(600 초) 이상
    }
    assert_ne!(host.state(), HostState::Disabled);
    ok_echo(&host);
}

#[test]
fn t_phost_flt_04_worker_killed_while_idle_is_detected_and_counted() {
    let (host, _, _) = new_host(PLUGIN, |_| {});
    ok_echo(&host);
    let HostState::Running { pid } = host.state() else { panic!("running") };
    let st = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).stdout(std::process::Stdio::null()).status().unwrap();
    assert!(st.success());
    // 다음 호출: 죽은 워커를 감지하고 새로 시작해 성공
    ok_echo(&host);
    assert_eq!(host.starts(), 2);
}

#[test]
fn t_phost_flt_05_missing_worker_exe_is_start_error_not_a_fault() {
    let api = Arc::new(FakeApi::default());
    let cfg = HostConfig::new(r"C:\no\such\mam-plugin-worker.exe".into(), PLUGIN.into());
    let host = PluginHost::new(plugin_id("com.example.test"), cfg, api, FakeClock::new());
    for _ in 0..5 {
        let e = run(&host, "echo", json!(1)).unwrap_err();
        assert!(matches!(e, InvokeError::Start(_)), "{e:?}");
    }
    assert_eq!(host.state(), HostState::Idle, "실행 파일 문제는 플러그인 결함이 아니다 — 비활성되지 않는다");
}

/// 가짜 워커(.cmd): `ready` 를 보낸 뒤 `body` 줄들을 내보내고 stdin EOF 까지 대기. 반환된 임시 디렉터리를 살려 둘 것
fn fake_worker(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake-worker.cmd");
    let script = format!("@echo off\r\necho {{\"t\":\"ready\",\"v\":1,\"commands\":[\"a\"],\"providers\":[]}}\r\n{body}\r\nmore >nul\r\n");
    std::fs::write(&path, script).unwrap();
    (dir, path)
}

fn fake_host(exe: std::path::PathBuf, max_line: u64) -> PluginHost {
    let mut cfg = HostConfig::new(exe, String::new());
    cfg.limits.max_line_bytes = max_line;
    PluginHost::new(plugin_id("com.example.test"), cfg, Arc::new(FakeApi::default()), FakeClock::new())
}

#[test]
fn t_phost_flt_06_malformed_and_unknown_worker_messages_kill_the_worker() {
    for body in ["echo this is not json", "echo {\"t\":\"nonsense\"}", "echo {\"t\":\"result\",\"id\":999,\"ok\":true}"] {
        let (_dir, exe) = fake_worker(body);
        let host = fake_host(exe, 1 << 20);
        let e = run(&host, "a", json!(null)).unwrap_err();
        assert!(matches!(e, InvokeError::Protocol(_)), "{body}: {e:?}");
        assert_eq!(host.state(), HostState::Idle);
    }
}

#[test]
fn t_phost_flt_07_oversized_line_kills_the_worker() {
    // Arrange: 줄 상한 1000 바이트, 가짜 워커가 2000 자 줄을 보낸다
    let (_dir, exe) = fake_worker(&format!("echo {}", "x".repeat(2000)));
    let host = fake_host(exe, 1000);
    // Act / Assert
    assert_eq!(run(&host, "a", json!(null)).unwrap_err(), InvokeError::Oversize);
    assert_eq!(host.state(), HostState::Idle);
}

#[test]
fn t_phost_flt_08_wrong_first_message_is_a_protocol_error() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("bad.cmd");
    std::fs::write(&exe, "@echo off\r\necho {\"t\":\"ready\",\"v\":99,\"commands\":[\"a\"],\"providers\":[]}\r\nmore >nul\r\n").unwrap();
    let host = fake_host(exe, 1 << 20);
    assert!(matches!(run(&host, "a", json!(null)).unwrap_err(), InvokeError::Protocol(m) if m.contains("99")));
}

#[test]
fn t_phost_flt_12_empty_worker_path_never_spawns_and_is_a_start_error() {
    // Arrange: 워커를 못 찾은 호출자는 빈 경로를 준다
    let api = Arc::new(FakeApi::default());
    let host = PluginHost::new(plugin_id("com.example.test"), HostConfig::new(std::path::PathBuf::new(), PLUGIN.into()), api, FakeClock::new());
    // Act / Assert
    for _ in 0..3 {
        let e = run(&host, "echo", json!(1)).unwrap_err();
        assert!(matches!(&e, InvokeError::Start(k) if k.contains("not found")), "{e:?}");
    }
    assert_eq!(host.starts(), 0);
    assert_eq!(host.state(), HostState::Idle);
}
