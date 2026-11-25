//! 자원 한도·타임아웃·폭주 시험. 분류: External(워커 폭주) / Boundary(한도) / Time(타임아웃) / Concurrency(재시작 후 정상)
mod common;

use std::time::{Duration, Instant};

use common::*;
use mam_plugin_host::{HostState, InvokeError};
use serde_json::json;

#[test]
fn t_phost_lim_01_timeout_kills_worker_and_next_call_restarts() {
    // Arrange: CPU 한도를 끄고(0) 호스트 타임아웃만으로 끊는다
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.cpu_ms = 0);
    ok_echo(&host);
    let HostState::Running { pid: old } = host.state() else {
        panic!("worker should be running")
    };
    // Act
    let t = Instant::now();
    let e = host
        .invoke_with_timeout(
            mam_plugin_host::InvokeKind::Command,
            "spin",
            json!(null),
            mam_plugin_host::InvokeScope::empty(),
            Duration::from_millis(500),
        )
        .unwrap_err();
    // Assert: 타임아웃 후 종료, 다음 호출은 새 워커(다른 PID)로 성공
    assert_eq!(e, InvokeError::Timeout);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(host.state(), HostState::Idle);
    ok_echo(&host);
    let HostState::Running { pid: new } = host.state() else {
        panic!("restarted")
    };
    assert_ne!(old, new);
    assert_eq!(host.starts(), 2);
}

#[test]
fn t_phost_lim_02_cpu_interrupt_stops_infinite_loop_before_the_call_timeout() {
    // Arrange: CPU 300 ms, 호출 타임아웃 10 s
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.cpu_ms = 300);
    // Act
    let t = Instant::now();
    let e = run(&host, "spin", json!(null)).unwrap_err();
    // Assert: 워커 자신이 interrupt 로 멈추고 `Limit` 로 보고 — 호스트 타임아웃(10 s)보다 훨씬 빠르다
    assert!(
        matches!(&e, InvokeError::Limit(m) if m.contains("cpu")),
        "{e:?}"
    );
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    ok_echo(&host);
    assert_eq!(host.starts(), 2, "한도 위반 워커는 버리고 새로 시작");
}

#[test]
fn t_phost_lim_03_cpu_limit_excludes_time_blocked_in_host_calls() {
    // Arrange: 3 라운드 x (호스트 대기 400 ms + JS 100 ms) = 벽시계 1.5 s, CPU 0.3 s. 한도 500 ms
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.cpu_ms = 500);
    // Act / Assert: 대기 시간이 CPU 로 세어지지 않으므로 성공
    assert_eq!(
        run(
            &host,
            "busy_with_calls",
            json!({"rounds": 3, "sleepMs": 400, "busyMs": 100})
        )
        .unwrap(),
        json!("done")
    );
    // 같은 형태에서 JS 구간 합이 한도를 넘으면(8 x 100 ms > 500 ms) 중단된다 — 호스트 호출 사이에도 interrupt 가 동작
    let e = run(
        &host,
        "busy_with_calls",
        json!({"rounds": 8, "sleepMs": 1, "busyMs": 100}),
    )
    .unwrap_err();
    assert!(matches!(e, InvokeError::Limit(_)), "{e:?}");
}

#[test]
fn t_phost_lim_04_memory_bomb_is_a_limit_error_host_survives() {
    // Arrange: 힙 16 MiB
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.memory_bytes = 16 << 20);
    // Act
    let e = run(&host, "bomb", json!(null)).unwrap_err();
    // Assert
    assert!(
        matches!(&e, InvokeError::Limit(m) if m.contains("memory")),
        "{e:?}"
    );
    ok_echo(&host);
    assert_eq!(host.starts(), 2);
}

#[test]
fn t_phost_lim_05_output_flood_kills_worker() {
    // Arrange: 호출당 메시지 500 개까지
    let (host, api, _) = new_host(PLUGIN, |c| c.max_messages_per_call = 500);
    // Act
    let e = run(&host, "flood", json!(null)).unwrap_err();
    // Assert: 폭주 오류, 로그는 상한까지만 전달, 다음 호출은 새 워커
    assert_eq!(e, InvokeError::Flood);
    assert!(api.logs.lock().unwrap().len() <= 500);
    assert_eq!(host.state(), HostState::Idle);
    ok_echo(&host);
}

#[test]
fn t_phost_lim_06_infinite_host_call_loop_is_stopped_by_message_cap_or_timeout() {
    // 호스트 호출을 끝없이 반복해도(대기 시간은 CPU 로 안 세어짐) 무한히 이어지지 않는다
    let src = "mam.commands.register('loop', () => { for (;;) mam.host.call('echo', 1); });";
    // 전체 워크스페이스 부하 시 기본 CPU 상한(2 s)이 메시지 상한보다 먼저 걸리지 않도록 CPU 상한을 올린다 (t_pgw_41 과 같은 조치)
    let (host, _, _) = new_host(src, |c| {
        c.max_messages_per_call = 2000;
        c.limits.cpu_ms = 120_000;
    });
    let e = run(&host, "loop", json!(null)).unwrap_err();
    assert!(
        matches!(e, InvokeError::Flood | InvokeError::Timeout),
        "{e:?}"
    );
}

// 분류: Boundary(줄 상한) / Validation(too_large)

#[test]
fn t_phost_lim_12_oversize_host_call_params_are_rejected_in_the_worker_and_it_survives() {
    // Arrange: 줄 상한 1 MiB, 인자 2 MiB — 호스트 API 까지 가지 않아야 한다
    let src = "mam.commands.register('bigparams', () => { try { mam.host.call('echo', 'x'.repeat(2 * 1024 * 1024)); return 'no error'; } catch (e) { return { code: e.code }; } });\nmam.commands.register('echo', (p) => p);";
    let (host, api, _) = new_host(src, |c| c.limits.max_line_bytes = 1 << 20);
    // Act
    let t = Instant::now();
    let r = run(&host, "bigparams", json!(null)).unwrap();
    // Assert: 워커 안에서 too_large 로 거절, 호스트 호출 0건, 같은 워커가 계속 산다
    assert_eq!(r, json!({"code": "too_large"}));
    assert!(api.calls.lock().unwrap().is_empty());
    assert!(t.elapsed() < Duration::from_secs(5));
    assert_eq!(run(&host, "echo", json!(1)).unwrap(), json!(1));
    assert_eq!(host.starts(), 1);
}

#[test]
fn t_phost_lim_13_oversize_result_is_too_large_and_worker_survives() {
    // Arrange: 결과 2 MiB, 줄 상한 1 MiB — 엔진이 파싱 전에 거절
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.max_line_bytes = 1 << 20);
    // Act
    let e = run(&host, "bigresult", json!(null)).unwrap_err();
    // Assert
    assert!(
        matches!(&e, InvokeError::Plugin { code, .. } if code == "too_large"),
        "{e:?}"
    );
    ok_echo(&host);
    assert_eq!(host.starts(), 1);
}

// 분류: Boundary(프로세스 메모리 상한) / External(워커 폭주)

#[test]
fn t_phost_lim_14_process_memory_cap_stops_runaway_worker_without_hanging() {
    // Arrange: QuickJS 한도를 꺼서(0) Job 의 프로세스 메모리 상한(256 MiB)만 남긴다
    let (host, _, _) = new_host(PLUGIN, |c| c.limits.memory_bytes = 0);
    // Act
    let t = Instant::now();
    let e = run(&host, "bomb", json!(null)).unwrap_err();
    // Assert: 걸리지 않고 한도/크래시로 보고되며 호스트는 산다
    assert!(
        matches!(e, InvokeError::Limit(_) | InvokeError::Crashed),
        "{e:?}"
    );
    assert!(t.elapsed() < Duration::from_secs(20), "{:?}", t.elapsed());
    ok_echo(&host);
}
