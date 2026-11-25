//! 사이드카가 손자 프로세스를 띄우는 shim 일 때, 취소·타임아웃이 트리 전체를 죽이고 즉시 돌아오는지.
//! 분류: Concurrency (실행 중 취소) / Time (타임아웃) / External (실제 프로세스 트리) — Windows 전용.
//!
//! shim = `cmd /c ping -n 60 127.0.0.1` (cmd 가 ping 을 자식으로 띄우고 기다린다 — Chocolatey shim 과 같은 구조).
//! 테스트가 같은 프로세스에서 자식 트리를 열거하므로 직렬 실행한다.
#![cfg(windows)]

use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mam_media::MediaError;
use mam_media::tool::{CancellableProcessRunner, CommandRunner, ProcessRunner, RunOptions, Tool};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};

static SERIAL: Mutex<()> = Mutex::new(());
/// 종료 요청 뒤 프로세스가 실제로 사라질 때까지 기다려 주는 총 유예 시간 (종료는 비동기라 느린 러너에서 지연된다).
const EXIT_GRACE: Duration = Duration::from_secs(3);

fn shim_args() -> Vec<OsString> {
    ["/c", "ping -n 60 127.0.0.1"].map(OsString::from).to_vec()
}

/// 이 테스트 프로세스의 모든 후손 PID.
fn descendants() -> Vec<u32> {
    let mut all: Vec<(u32, u32)> = Vec::new(); // (pid, parent)
    // SAFETY: ToolHelp 스냅샷 열거 후 핸들을 닫는다.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        assert_ne!(snap, INVALID_HANDLE_VALUE);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snap, &mut e) != 0;
        while more {
            all.push((e.th32ProcessID, e.th32ParentProcessID));
            more = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    let mut found = vec![std::process::id()];
    let mut i = 0;
    while i < found.len() {
        let p = found[i];
        for &(pid, parent) in &all {
            if parent == p && !found.contains(&pid) {
                found.push(pid);
            }
        }
        i += 1;
    }
    found.remove(0);
    found
}

/// `deadline` 까지 종료를 기다린 뒤에도 살아 있으면 true (= 진짜 누수). 핸들을 못 열면 이미 사라진 것.
fn is_alive_after(pid: u32, deadline: Instant) -> bool {
    let wait_ms = deadline
        .saturating_duration_since(Instant::now())
        .as_millis() as u32;
    // SAFETY: 동기화 핸들을 열어 종료 신호를 기다리고 닫는다.
    unsafe {
        let h = OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if h.is_null() {
            return false;
        }
        let r = WaitForSingleObject(h, wait_ms);
        CloseHandle(h);
        r == WAIT_TIMEOUT
    }
}

/// 후손이 2개 이상(cmd + ping) 생길 때까지 기다린 뒤 그 목록을 돌려준다 (상한 5초).
fn wait_for_tree() -> Vec<u32> {
    for _ in 0..100 {
        let d = descendants();
        if d.len() >= 2 {
            return d;
        }
        thread::sleep(Duration::from_millis(50));
    }
    descendants()
}

/// 유예 시간(`EXIT_GRACE`) 안에 끝나지 않은 프로세스를 누수로 돌려주고 PID 로만 정리한다 (실패한 단언이 고아를 남기지 않게).
fn reap(pids: &[u32]) -> Vec<u32> {
    let deadline = Instant::now() + EXIT_GRACE;
    let alive: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|p| is_alive_after(*p, deadline))
        .collect();
    for p in &alive {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &p.to_string(), "/T", "/F"])
            .output();
    }
    alive
}

fn opts(timeout: Duration) -> RunOptions {
    RunOptions {
        timeout,
        max_stdout: 1 << 20,
    }
}

#[test]
fn t_media_tree_01_cancel_kills_grandchild_and_returns_promptly() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Arrange
    let flag = Arc::new(AtomicBool::new(false));
    let runner = CancellableProcessRunner::new(Arc::clone(&flag));
    let watcher = {
        let flag = Arc::clone(&flag);
        thread::spawn(move || {
            let tree = wait_for_tree();
            flag.store(true, Ordering::SeqCst);
            tree
        })
    };
    let started = Instant::now();
    // Act
    let r = runner.run(
        Tool::Ffmpeg,
        Path::new("cmd"),
        &shim_args(),
        &opts(Duration::from_secs(60)),
    );
    let took = started.elapsed();
    let tree = watcher.join().unwrap();
    let leaked = reap(&tree);
    // Assert
    assert!(tree.len() >= 2, "shim tree not observed: {tree:?}");
    assert!(matches!(r, Err(MediaError::Cancelled { .. })), "{r:?}");
    assert!(took < Duration::from_secs(3), "took {took:?}");
    assert!(leaked.is_empty(), "orphans survived cancel: {leaked:?}");
}

#[test]
fn t_media_tree_02_timeout_kills_grandchild_and_returns_promptly() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Arrange
    let watcher = thread::spawn(wait_for_tree);
    let started = Instant::now();
    // Act
    let r = ProcessRunner.run(
        Tool::Ffmpeg,
        Path::new("cmd"),
        &shim_args(),
        &opts(Duration::from_millis(1500)),
    );
    let took = started.elapsed();
    let tree = watcher.join().unwrap();
    let leaked = reap(&tree);
    // Assert
    assert!(tree.len() >= 2, "shim tree not observed: {tree:?}");
    assert!(matches!(r, Err(MediaError::Timeout { .. })), "{r:?}");
    assert!(took < Duration::from_secs(4), "took {took:?}");
    assert!(leaked.is_empty(), "orphans survived timeout: {leaked:?}");
}

#[test]
fn t_media_tree_03_normal_exit_still_returns_output_and_code() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Arrange / Act: shim 이 손자(echo 가 아닌 ping 1회)를 띄우고 정상 종료
    let out = ProcessRunner
        .run(
            Tool::Ffmpeg,
            Path::new("cmd"),
            &["/c", "ping -n 1 127.0.0.1"].map(OsString::from),
            &opts(Duration::from_secs(20)),
        )
        .unwrap();
    // Assert
    assert_eq!(out.code, Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("127.0.0.1"));
}
