//! 호스트 비정상 종료 시 워커가 고아로 남지 않는다 (T-phost-orph-*) — 분류: Happy(크래시 후 워커 소멸) / External(자식 호스트 무응답 시 시험이 멈추지 않고 실패).
//! Windows 전용 — Job Object(KILL_ON_JOB_CLOSE) 는 Windows 기능이다.
//!
//! 방식: 이 시험 실행 파일을 자식 "호스트 프로세스" 로 다시 실행한다(`--ignored --exact` + 환경변수 `MAM_ORPHAN_ROLE=host`).
//! 자식 역할 시험은 `#[ignore]` 라 일반 실행에는 돌지 않는다. 자식은 워커를 띄우고 PID 를 출력한 뒤 부모의 신호(stdin 한 줄)를
//! 기다렸다가(상한 있음, 넘으면 그냥 abort) 정리 없이 `abort()` 한다. 부모는 신호 전에 워커 핸들을 열어 두고,
//! 모든 대기(PID 수신·자식 종료·워커 종료)에 제한 시간을 둔다 — 멈추지 않고 실패한다. sleep 폴링 없음.
#![cfg(windows)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::{new_host, ok_echo};
use mam_plugin_host::HostState;
use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

const ROLE_ENV: &str = "MAM_ORPHAN_ROLE";
/// 자식이 워커를 띄워 PID 를 보고하기까지(첫 실행은 워커 빌드 포함)
const PID_WAIT: Duration = Duration::from_secs(120);
const CHILD_EXIT_WAIT: Duration = Duration::from_secs(30);
const WORKER_EXIT_MS: u32 = 15_000;

/// 자식 "호스트 프로세스" 본체 — `--ignored` 로만 실행되며, 환경변수가 없으면 아무것도 하지 않는다
#[test]
#[ignore = "child role of t_phost_orph_01 (spawned by it with MAM_ORPHAN_ROLE=host)"]
fn t_phost_orph_child_role_host_that_aborts() {
    if std::env::var(ROLE_ENV).as_deref() != Ok("host") {
        return;
    }
    let (host, _, _) = new_host(common::PLUGIN, |_| {});
    ok_echo(&host); // 워커 시작
    let HostState::Running { pid } = host.state() else { std::process::abort() };
    println!("WORKER_PID={pid}");
    std::io::stdout().flush().ok();
    // 부모가 워커 핸들을 연 뒤에만 죽는다. 신호가 안 와도 상한 뒤에는 죽는다 (영구 대기 없음)
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(30));
    std::process::abort(); // Drop·shutdown 없이 즉시 종료
}

/// T-phost-orph-01: 호스트가 정리 없이 죽으면 워커도 제한 시간 안에 사라진다
#[test]
fn t_phost_orph_01_worker_dies_when_host_process_aborts() {
    // Arrange: 자식 호스트를 띄우고 워커 PID 를 (제한 시간 안에) 받는다
    let exe = std::env::current_exe().expect("test exe");
    let mut child = Command::new(exe)
        .args(["--ignored", "--exact", "t_phost_orph_child_role_host_that_aborts", "--nocapture", "--test-threads=1"])
        .env(ROLE_ENV, "host")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child host");
    let out = child.stdout.take().expect("stdout");
    let (pid_tx, pid_rx) = mpsc::channel::<u32>();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            // libtest 가 "test 이름 ... " 를 같은 줄에 먼저 찍으므로 접두 일치가 아니라 마커 위치로 찾는다
            if let Some(p) = line.split_once("WORKER_PID=").and_then(|(_, p)| p.trim().parse().ok()) {
                let _ = pid_tx.send(p);
                return;
            }
        }
    });
    let Ok(pid) = pid_rx.recv_timeout(PID_WAIT) else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("child host did not report a worker pid within {PID_WAIT:?}");
    };
    // SAFETY: 유효한 PID 로 동기화 권한 핸들만 연다. 아래에서 한 번 닫는다.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if handle.is_null() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("worker {pid} must be alive and openable before the host dies");
    }

    // Act: 자식 호스트를 정리 없이 종료시킨다
    child.stdin.take().expect("stdin").write_all(b"go\n").expect("signal child");
    let (st_tx, st_rx) = mpsc::channel();
    let mut waiter = child;
    let kill_handle = waiter.id();
    std::thread::spawn(move || {
        let _ = st_tx.send(waiter.wait());
    });
    let exited = st_rx.recv_timeout(CHILD_EXIT_WAIT);

    // Assert: 호스트는 비정상 종료했고, 워커 핸들이 제한 시간 안에 신호 상태가 된다
    // SAFETY: 위에서 연 유효한 핸들.
    let waited = unsafe { WaitForSingleObject(handle, WORKER_EXIT_MS) };
    unsafe { CloseHandle(handle) };
    if exited.is_err() {
        // 자식이 안 죽었다 — 이 시험이 띄운 PID 만 정리하고 실패한다
        let _ = Command::new("taskkill").args(["/PID", &kill_handle.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        panic!("child host {kill_handle} did not exit within {CHILD_EXIT_WAIT:?}");
    }
    assert!(!exited.unwrap().expect("wait child host").success(), "child host should have aborted");
    assert_eq!(waited, WAIT_OBJECT_0, "worker {pid} survived its host (orphan)");
}
