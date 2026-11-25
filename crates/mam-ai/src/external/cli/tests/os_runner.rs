//! 실제 프로세스 실행기 시험 (Windows) — 시스템 도구(`cmd`, `ping`, `findstr`)만 쓰고 `claude` 는 쓰지 않는다.
//! 분류: Happy / External(종료·제한 시간) / Business rule(PID 종료, 파이프 stdin)
#![cfg(windows)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

use crate::external::cli::{CliCommand, Exec, Flow, OsRunner, RunEnd};

fn sys(name: &str) -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()))
        .join("System32")
        .join(name)
}

fn cmd(program: PathBuf, args: &[&str], stdin: Option<&[u8]>) -> CliCommand {
    CliCommand {
        program,
        args: args.iter().map(OsString::from).collect(),
        cwd: None,
        stdin: stdin.map(<[u8]>::to_vec),
    }
}

/// PID 가 더 이상 살아 있지 않은가 (핸들을 못 열거나 종료 신호 상태).
fn is_gone(pid: u32) -> bool {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    // SAFETY: 읽기 전용 질의 — 핸들은 즉시 닫는다.
    unsafe {
        let h = OpenProcess(SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return true;
        }
        // 종료는 비동기 — 느린 러너를 위해 최대 3초 유예 (살아 있으면 WAIT_TIMEOUT 이라 false).
        let r = WaitForSingleObject(h, 3000);
        CloseHandle(h);
        r == 0
    }
}

#[test]
fn t_aicli_os_01_streams_stdout_lines_and_exit_code() {
    let exec = Exec::new(Arc::new(OsRunner));
    let mut lines = Vec::new();

    let r = exec
        .run_lines(
            &cmd(sys("cmd.exe"), &["/C", "echo one&echo two&exit 3"], None),
            Duration::from_secs(10),
            |l| {
                lines.push(l.to_string());
                Flow::Continue
            },
        )
        .unwrap();

    assert_eq!(r.end, RunEnd::Exited(Some(3)));
    assert_eq!(lines, vec!["one", "two"]);
}

#[test]
fn t_aicli_os_02_stdin_is_piped_and_closed() {
    let exec = Exec::new(Arc::new(OsRunner));

    let out = exec
        .capture(
            &cmd(
                sys("findstr.exe"),
                &["/R", "^"],
                Some("가나다 hello\n둘째 줄\n".as_bytes()),
            ),
            Duration::from_secs(10),
        )
        .unwrap();

    assert!(matches!(out.end, RunEnd::Exited(_)));
    assert!(out.stdout.contains("hello"), "{:?}", out.stdout);
}

#[test]
fn t_aicli_os_03_timeout_kills_the_process_by_pid() {
    let exec = Exec::new(Arc::new(OsRunner));
    let started = Instant::now();

    let r = exec
        .run_lines(
            &cmd(sys("ping.exe"), &["-n", "30", "127.0.0.1"], None),
            Duration::from_millis(400),
            |_| Flow::Continue,
        )
        .unwrap();

    assert_eq!(r.end, RunEnd::Timeout);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(is_gone(r.pid), "시간 초과 뒤 프로세스가 남아 있다");
    assert_eq!(exec.active_count(), 0);
}

#[test]
fn t_aicli_os_04_stop_from_the_line_callback_kills_the_process() {
    let exec = Exec::new(Arc::new(OsRunner));

    let r = exec
        .run_lines(
            &cmd(sys("ping.exe"), &["-n", "30", "127.0.0.1"], None),
            Duration::from_secs(20),
            |l| {
                if l.contains("127.0.0.1") {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            },
        )
        .unwrap();

    assert_eq!(r.end, RunEnd::Stopped);
    assert!(is_gone(r.pid));
}

#[test]
fn t_aicli_os_05_kill_all_ends_a_running_process_from_another_thread() {
    let exec = Arc::new(Exec::new(Arc::new(OsRunner)));
    let e2 = Arc::clone(&exec);
    let t = std::thread::spawn(move || {
        e2.run_lines(
            &cmd(sys("ping.exe"), &["-n", "30", "127.0.0.1"], None),
            Duration::from_secs(30),
            |_| Flow::Continue,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while exec.active_count() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    exec.kill_all();
    let r = t.join().unwrap().unwrap();

    assert_eq!(r.end, RunEnd::Cancelled);
    assert!(is_gone(r.pid));
}

#[test]
fn t_aicli_os_06_missing_program_is_a_spawn_error() {
    let exec = Exec::new(Arc::new(OsRunner));
    let r = exec.run_lines(
        &cmd(PathBuf::from(r"C:\definitely\not\here.exe"), &[], None),
        Duration::from_secs(1),
        |_| Flow::Continue,
    );
    assert!(r.is_err());
}

#[test]
fn t_aicli_os_07_utf8_split_across_chunks_is_reassembled() {
    let mut carry = "한".as_bytes()[..2].to_vec();
    assert_eq!(super::super::os_runner::take_utf8(&mut carry), "");
    carry.extend_from_slice(&"한".as_bytes()[2..]);
    assert_eq!(super::super::os_runner::take_utf8(&mut carry), "한");
    let mut bad = vec![b'a', 0xFF, b'b'];
    assert_eq!(super::super::os_runner::take_utf8(&mut bad), "a\u{FFFD}b");
}
