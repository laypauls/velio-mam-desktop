//! `std::process` 기반 실행기 — 읽기 스레드가 출력 조각을 채널로 보내고, `next_event` 가 종료를 감시한다.
//! 종료는 PID 트리 종료(`taskkill /PID <pid> /T /F`) 후 `Child::kill`/`wait` 로 회수한다(좀비·고아 방지).

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::process::{CliCommand, CliProcess, CliRunner, ProcEvent};

/// 종료 감시 주기 — `next_event` 가 이 간격으로 `try_wait` 한다.
const WATCH_SLICE: Duration = Duration::from_millis(25);
/// 종료 직후 남은 출력을 모으는 최대 시간 (손자 프로세스가 파이프를 쥔 경우 무한 대기 방지).
const DRAIN_CAP: Duration = Duration::from_millis(300);

#[derive(Debug, Default, Clone, Copy)]
pub struct OsRunner;

impl CliRunner for OsRunner {
    fn spawn(&self, cmd: &CliCommand) -> io::Result<Box<dyn CliProcess>> {
        let mut c = Command::new(&cmd.program);
        c.args(&cmd.args)
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cmd.cwd {
            c.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = c.spawn()?;
        let (tx, rx) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            spawn_reader(out, tx.clone(), false);
        }
        if let Some(err) = child.stderr.take() {
            spawn_reader(err, tx.clone(), true);
        }
        drop(tx);
        if let (Some(mut stdin), Some(data)) = (child.stdin.take(), cmd.stdin.clone()) {
            // 큰 요청(base64 이미지)이 파이프 버퍼보다 커도 출력 읽기와 교착하지 않도록 별도 스레드에서 쓴다
            thread::spawn(move || {
                let _ = stdin.write_all(&data);
                let _ = stdin.flush();
            });
        }
        Ok(Box::new(OsProcess {
            pid: child.id(),
            child,
            rx,
            queue: VecDeque::new(),
            finished: false,
            watch_iterations: 0,
        }))
    }

    fn kill_pid(&self, pid: u32) {
        kill_tree(pid);
    }
}

fn spawn_reader(mut r: impl Read + Send + 'static, tx: Sender<ProcEvent>, is_err: bool) {
    thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let mut carry: Vec<u8> = Vec::new();
        loop {
            let n = match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            carry.extend_from_slice(&buf[..n]);
            let text = take_utf8(&mut carry);
            if !text.is_empty() {
                let ev = if is_err {
                    ProcEvent::Stderr(text)
                } else {
                    ProcEvent::Stdout(text)
                };
                if tx.send(ev).is_err() {
                    break;
                }
            }
        }
    });
}

/// `carry` 에서 유효한 UTF-8 접두를 꺼낸다. 끝에 잘린 멀티바이트는 다음 조각을 위해 남기고, 잘못된 바이트는 대체 문자로 바꾼다.
pub(crate) fn take_utf8(carry: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(s) => {
                out.push_str(s);
                carry.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(&String::from_utf8_lossy(&carry[..valid]));
                match e.error_len() {
                    None => {
                        carry.drain(..valid);
                        return out; // 불완전한 끝 — 다음 읽기에서 이어진다
                    }
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        carry.drain(..valid + bad);
                    }
                }
            }
        }
    }
}

struct OsProcess {
    pid: u32,
    child: Child,
    rx: Receiver<ProcEvent>,
    queue: VecDeque<ProcEvent>,
    finished: bool,
    /// `next_event` 감시 루프를 돈 횟수 (두 파이프가 닫힌 채 살아 있는 프로세스에서 바쁜 대기를 하지 않는지 시험용)
    watch_iterations: u64,
}

impl OsProcess {
    /// 종료가 확인된 뒤 남은 출력을 모아 `Exit` 를 뒤에 붙인다.
    fn drain_after_exit(&mut self, code: Option<i32>) {
        self.finished = true;
        let deadline = Instant::now() + DRAIN_CAP;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.rx.recv_timeout(left) {
                Ok(ev) => self.queue.push_back(ev),
                Err(_) => break, // 모든 읽기 스레드 종료(Disconnected) 또는 기한
            }
        }
        self.queue.push_back(ProcEvent::Exit(code));
    }
}

impl CliProcess for OsProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn next_event(&mut self, wait: Duration) -> ProcEvent {
        if let Some(ev) = self.queue.pop_front() {
            return ev;
        }
        if self.finished {
            return ProcEvent::Idle;
        }
        let deadline = Instant::now() + wait;
        let mut pipes_closed = false;
        loop {
            let slice = deadline
                .saturating_duration_since(Instant::now())
                .min(WATCH_SLICE);
            match self.rx.recv_timeout(slice) {
                Ok(ev) => return ev,
                Err(RecvTimeoutError::Timeout) => {}
                // 두 파이프가 모두 닫혔다 - `recv_timeout` 이 즉시 돌아오므로 아래에서 직접 쉰다
                Err(RecvTimeoutError::Disconnected) => pipes_closed = true,
            }
            self.watch_iterations += 1;
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.drain_after_exit(status.code());
                    return self.queue.pop_front().unwrap_or(ProcEvent::Idle);
                }
                Ok(None) => {}
                Err(_) => {
                    self.drain_after_exit(None);
                    return self.queue.pop_front().unwrap_or(ProcEvent::Idle);
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return ProcEvent::Idle;
            }
            if pipes_closed {
                thread::sleep(left.min(WATCH_SLICE)); // 출력은 없고 프로세스만 살아 있다 - 바쁜 루프 금지
            }
        }
    }

    fn kill(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        kill_tree(self.pid);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for OsProcess {
    fn drop(&mut self) {
        if !self.finished && matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

/// PID 의 프로세스 트리를 종료한다. 이름으로 죽이지 않는다.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 두 파이프를 닫고 몇 초 더 사는 시스템 프로세스.
    fn quiet_sleeper() -> Child {
        #[cfg(windows)]
        let mut c = {
            use std::os::windows::process::CommandExt;
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
            let mut c = Command::new(format!(r"{root}\System32\ping.exe"));
            c.args(["-n", "6", "127.0.0.1"]).creation_flags(0x0800_0000);
            c
        };
        #[cfg(not(windows))]
        let mut c = {
            let mut c = Command::new("sleep");
            c.arg("5");
            c
        };
        c.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    // 분류: Concurrency / Boundary — 출력 채널이 끊긴 채 프로세스가 살아 있다 (회귀: 마감까지 바쁜 루프)
    #[test]
    fn t_aicli_os_20_closed_pipes_with_a_live_process_wait_without_spinning() {
        // Arrange: 송신단이 모두 사라진 채널 + 살아 있는 자식
        let (tx, rx) = mpsc::channel::<ProcEvent>();
        drop(tx);
        let child = quiet_sleeper();
        let mut p = OsProcess {
            pid: child.id(),
            child,
            rx,
            queue: VecDeque::new(),
            finished: false,
            watch_iterations: 0,
        };
        let started = Instant::now();

        // Act
        let ev = p.next_event(Duration::from_millis(400));

        // Assert: 제한 시간 동안 기다렸고(조기 반환 아님), 반복 횟수는 WATCH_SLICE 단위로 제한된다
        assert!(matches!(ev, ProcEvent::Idle));
        assert!(started.elapsed() >= Duration::from_millis(350));
        assert!(
            p.watch_iterations <= 400 / 25 + 4,
            "busy loop: {} iterations",
            p.watch_iterations
        );
        p.kill();
    }
}
