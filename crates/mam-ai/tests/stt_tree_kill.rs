//! STT 사이드카가 손자 프로세스를 띄우는 shim 일 때 취소·타임아웃이 트리 전체를 죽이는지 (Windows 전용).
//! 분류: Concurrency (실행 중 취소) / Time (타임아웃) / External (실제 프로세스 트리).
//! `mam-media/tests/tree_kill.rs` 와 같은 구조 — 두 crate 가 서로 의존하지 않아 헬퍼를 복제했다.
#![cfg(windows)]

use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use mam_ai::AiError;
use mam_ai::local::stt::{CancelToken, CommandRunner, ProcessRunner, RunOptions, Tool};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

static SERIAL: Mutex<()> = Mutex::new(());
const STILL_ACTIVE: u32 = 259;

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

fn is_alive(pid: u32) -> bool {
    // SAFETY: 조회 전용 핸들을 열고 닫는다.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        ok != 0 && code == STILL_ACTIVE
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

/// 남은 프로세스를 PID 로만 정리한다 (실패한 단언이 고아를 남기지 않게).
fn reap(pids: &[u32]) -> Vec<u32> {
    let alive: Vec<u32> = pids.iter().copied().filter(|p| is_alive(*p)).collect();
    for p in &alive {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &p.to_string(), "/T", "/F"])
            .output();
    }
    alive
}

fn run_opts(timeout: Duration, cancel: Option<CancelToken>) -> RunOptions {
    RunOptions {
        timeout,
        cancel,
        low_priority: true,
    }
}

#[test]
fn t_ai_tree_01_cancel_kills_grandchild_and_returns_promptly() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Arrange
    let token = CancelToken::new();
    let watcher = {
        let token = token.clone();
        thread::spawn(move || {
            let tree = wait_for_tree();
            token.cancel();
            tree
        })
    };
    let started = Instant::now();
    // Act
    let r = ProcessRunner.run(
        Tool::WhisperCli,
        Path::new("cmd"),
        &shim_args(),
        &run_opts(Duration::from_secs(60), Some(token)),
    );
    let took = started.elapsed();
    let tree = watcher.join().unwrap();
    let leaked = reap(&tree);
    // Assert
    assert!(tree.len() >= 2, "shim tree not observed: {tree:?}");
    assert!(matches!(r, Err(AiError::Cancelled { .. })), "{r:?}");
    assert!(took < Duration::from_secs(3), "took {took:?}");
    assert!(leaked.is_empty(), "orphans survived cancel: {leaked:?}");
}

#[test]
fn t_ai_tree_02_timeout_kills_grandchild_and_returns_promptly() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Arrange
    let watcher = thread::spawn(wait_for_tree);
    let started = Instant::now();
    // Act
    let r = ProcessRunner.run(
        Tool::WhisperCli,
        Path::new("cmd"),
        &shim_args(),
        &run_opts(Duration::from_millis(1500), None),
    );
    let took = started.elapsed();
    let tree = watcher.join().unwrap();
    let leaked = reap(&tree);
    // Assert
    assert!(tree.len() >= 2, "shim tree not observed: {tree:?}");
    assert!(matches!(r, Err(AiError::Timeout { .. })), "{r:?}");
    assert!(took < Duration::from_secs(4), "took {took:?}");
    assert!(leaked.is_empty(), "orphans survived timeout: {leaked:?}");
}
