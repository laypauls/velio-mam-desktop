//! 사이드카 게이트 + 실행기: 경로 기준 차단·종료 (USB 꺼내기 준비).
//! 분류: Business rule (차단 경로는 띄우지 않음) / Concurrency (실행 중 종료·대기) / External (실제 프로세스)

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use mam_media::tool::{CancellableProcessRunner, CommandRunner, RunOptions, Tool};
use mam_media::{MediaError, SidecarGate};

fn opts() -> RunOptions {
    RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 1 << 20,
    }
}

fn runner(gate: &Arc<SidecarGate>) -> CancellableProcessRunner {
    CancellableProcessRunner::new(Arc::new(AtomicBool::new(false))).with_gate(Arc::clone(gate))
}

#[test]
fn t_media_gate_01_blocked_prefix_is_refused_without_spawning() {
    // Arrange - 존재하지 않는 프로그램: 띄웠다면 Io 오류가 된다
    let gate = SidecarGate::new();
    gate.block(r"E:\");
    let args: Vec<OsString> = vec!["-i".into(), r"E:\data\a.mp4".into()];

    // Act
    let err = runner(&gate)
        .run(
            Tool::Ffmpeg,
            Path::new("Z:/no/such/ffmpeg-binary"),
            &args,
            &opts(),
        )
        .unwrap_err();

    // Assert
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    assert_eq!(gate.running_under(r"E:\"), 0, "nothing stays registered");
}

#[cfg(windows)]
#[test]
fn t_media_gate_02_cancel_under_kills_the_running_child_by_pid_and_releases_waiters() {
    // Arrange - 30초 걸리는 프로세스(ping 30회). ping 은 낯선 인자를 거부하므로 게이트 접두는 호출 인자 중 하나(대상 주소)를 쓴다
    let gate = SidecarGate::new();
    let r = runner(&gate);
    let args: Vec<OsString> = ["-n", "30", "127.0.0.1"].map(OsString::from).to_vec();
    let prefix = "127.0.0.1";
    let started = Instant::now();
    let worker =
        std::thread::spawn(move || r.run(Tool::Ffprobe, Path::new("ping"), &args, &opts()));

    // Act: 프로세스가 뜰 때까지 기다렸다가 종료 요청
    let mut pids = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while pids.is_empty() && Instant::now() < deadline {
        pids = gate.cancel_under(prefix);
        std::thread::sleep(Duration::from_millis(20));
    }
    let released = gate.wait_none_under(prefix, Duration::from_secs(10));
    let result = worker.join().unwrap();

    // Assert
    assert!(
        !pids.is_empty() && pids[0] != 0,
        "child PID reported: {pids:?}"
    );
    assert!(released, "waiter released after the child was killed");
    assert!(
        matches!(result, Err(MediaError::Cancelled { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "did not wait for ping to finish"
    );
}

#[cfg(windows)]
#[test]
fn t_media_gate_03_unrelated_paths_run_normally_while_another_prefix_is_blocked() {
    // Arrange
    let gate = SidecarGate::new();
    gate.block(r"E:\");
    let args: Vec<OsString> = vec!["/C".into(), "exit".into(), "3".into()];

    // Act
    let out = runner(&gate)
        .run(Tool::Ffmpeg, Path::new("cmd"), &args, &opts())
        .unwrap();

    // Assert
    assert_eq!(out.code, Some(3));
}
