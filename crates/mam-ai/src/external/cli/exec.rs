//! 실행 허브 — 실행기 위에 줄 단위 스트리밍, 제한 시간, 취소, 활성 PID 추적(종료 경로가 PID 로 정리)을 얹는다.

use std::collections::HashSet;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::process::{CliCommand, CliProcess, CliRunner, ProcEvent};

/// 이벤트 대기 단위 — 취소·제한 시간 확인 주기.
const POLL: Duration = Duration::from_millis(50);
/// 한 줄 상한 (이보다 긴 줄은 버린다 — 결과 JSON 은 수 KB 다).
const MAX_LINE: usize = 4 * 1024 * 1024;
/// 오류 진단용으로 보관하는 stderr 끝부분 길이(문자).
const STDERR_KEEP: usize = 2048;
/// `capture` 가 보관하는 stdout 상한(바이트).
const CAPTURE_KEEP: usize = 64 * 1024;

/// 줄 콜백의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    /// 더 읽을 필요가 없다 — 프로세스를 종료한다 (예: 오프라인 조기 판정)
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Exited(Option<i32>),
    Timeout,
    Cancelled,
    /// 콜백이 `Stop` 을 돌려줘 종료했다
    Stopped,
}

#[derive(Debug, Clone)]
pub struct RunReport {
    pub end: RunEnd,
    pub pid: u32,
    /// stderr 끝부분 (진단 전용 — 사용자·로그에 그대로 내보내지 않는다)
    pub stderr: String,
}

#[derive(Debug, Clone)]
pub struct Captured {
    pub end: RunEnd,
    pub stdout: String,
    pub stderr: String,
}

type ActiveSet = Arc<Mutex<HashSet<u32>>>;

/// 실행 중 PID 를 추적하는 [`CliProcess`] 래퍼. 놓이면 PID 를 목록에서 빼고, 아직 살아 있으면 종료한다.
pub struct Tracked {
    proc: Box<dyn CliProcess>,
    active: ActiveSet,
}

impl Tracked {
    pub fn pid(&self) -> u32 {
        self.proc.pid()
    }
    pub fn next_event(&mut self, wait: Duration) -> ProcEvent {
        self.proc.next_event(wait)
    }
    pub fn kill(&mut self) {
        self.proc.kill();
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.proc.pid());
        self.proc.kill(); // 이미 끝났다면 아무 일도 하지 않는다
    }
}

pub struct Exec {
    runner: Arc<dyn CliRunner>,
    active: ActiveSet,
    cancel_all: AtomicBool,
}

impl Exec {
    pub fn new(runner: Arc<dyn CliRunner>) -> Self {
        Self {
            runner,
            active: Default::default(),
            cancel_all: AtomicBool::new(false),
        }
    }

    pub fn spawn(&self, cmd: &CliCommand) -> io::Result<Tracked> {
        if self.cancel_all.load(Ordering::SeqCst) {
            return Err(io::Error::other("cancelled"));
        }
        let proc = self.runner.spawn(cmd)?;
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(proc.pid());
        Ok(Tracked {
            proc,
            active: Arc::clone(&self.active),
        })
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_all.load(Ordering::SeqCst)
    }

    /// 종료 경로: 이후 새 실행을 막고, 실행 중인 모든 프로세스를 PID 로 종료한다.
    pub fn kill_all(&self) {
        self.cancel_all.store(true, Ordering::SeqCst);
        let pids: Vec<u32> = self
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .collect();
        for pid in pids {
            self.runner.kill_pid(pid);
        }
    }

    /// 지금 추적 중인 PID 수 (시험·진단용).
    pub fn active_count(&self) -> usize {
        self.active.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// `cmd` 를 실행하며 stdout 을 줄 단위로 `on_line` 에 넘긴다. 제한 시간·취소·`Stop` 이면 PID 로 종료한다.
    pub fn run_lines(
        &self,
        cmd: &CliCommand,
        timeout: Duration,
        mut on_line: impl FnMut(&str) -> Flow,
    ) -> io::Result<RunReport> {
        let mut p = self.spawn(cmd)?;
        let pid = p.pid();
        let deadline = Instant::now().checked_add(timeout);
        let mut buf = LineBuf::default();
        let mut stderr = String::new();
        let end = loop {
            if self.is_cancelled() {
                break RunEnd::Cancelled;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                break RunEnd::Timeout;
            }
            match p.next_event(POLL) {
                ProcEvent::Stdout(s) => {
                    let mut stop = false;
                    buf.push(&s, |line| {
                        if !stop && on_line(line) == Flow::Stop {
                            stop = true;
                        }
                    });
                    if stop {
                        break RunEnd::Stopped;
                    }
                }
                ProcEvent::Stderr(s) => keep_tail(&mut stderr, &s),
                ProcEvent::Exit(code) => {
                    let mut stop = false;
                    buf.finish(|line| {
                        if !stop && on_line(line) == Flow::Stop {
                            stop = true;
                        }
                    });
                    break RunEnd::Exited(code);
                }
                ProcEvent::Idle => {}
            }
        };
        if !matches!(end, RunEnd::Exited(_)) {
            p.kill();
        }
        Ok(RunReport { end, pid, stderr })
    }

    /// 짧은 명령(`--version`, `auth status`, `auth logout`)을 실행해 출력을 모은다.
    pub fn capture(&self, cmd: &CliCommand, timeout: Duration) -> io::Result<Captured> {
        let mut out = String::new();
        let report = self.run_lines(cmd, timeout, |line| {
            if out.len() < CAPTURE_KEEP {
                out.push_str(line);
                out.push('\n');
            }
            Flow::Continue
        })?;
        Ok(Captured {
            end: report.end,
            stdout: out,
            stderr: report.stderr,
        })
    }
}

fn keep_tail(buf: &mut String, add: &str) {
    buf.push_str(add);
    let n = buf.chars().count();
    if n > STDERR_KEEP {
        *buf = buf.chars().skip(n - STDERR_KEEP).collect();
    }
}

/// 조각 -> 줄. `\r\n` 의 `\r` 은 제거한다. 상한을 넘는 줄은 버린다.
#[derive(Default)]
struct LineBuf {
    cur: String,
    overflow: bool,
}

impl LineBuf {
    fn push(&mut self, chunk: &str, mut emit: impl FnMut(&str)) {
        let mut rest = chunk;
        while let Some(i) = rest.find('\n') {
            self.append(&rest[..i]);
            self.flush(&mut emit);
            rest = &rest[i + 1..];
        }
        self.append(rest);
    }

    fn append(&mut self, s: &str) {
        if self.overflow {
            return;
        }
        if self.cur.len() + s.len() > MAX_LINE {
            self.overflow = true;
            self.cur.clear();
            return;
        }
        self.cur.push_str(s);
    }

    fn flush(&mut self, emit: &mut impl FnMut(&str)) {
        if !self.overflow {
            let line = self.cur.trim_end_matches('\r');
            if !line.is_empty() {
                emit(line);
            }
        }
        self.cur.clear();
        self.overflow = false;
    }

    fn finish(&mut self, mut emit: impl FnMut(&str)) {
        self.flush(&mut emit);
    }
}
