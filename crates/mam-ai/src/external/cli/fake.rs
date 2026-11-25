//! 시험용 가짜 실행기 — 실제 `claude` 를 띄우지 않고 각본대로 출력·종료를 내보낸다. 시험 전용(운영 코드에서 쓰지 않는다).

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::process::{CliCommand, CliProcess, CliRunner, ProcEvent};

/// 각본의 한 단계.
#[derive(Debug, Clone)]
pub enum Step {
    Out(String),
    Err(String),
    Exit(Option<i32>),
    /// 이 단계에 도달하면 계속 `Idle`(대기)만 낸다 — 제한 시간·취소 시험용
    Hang,
    /// 깃발이 켜질 때까지 `Idle`, 켜지면 다음 단계로 — "사용자가 브라우저에서 승인한 뒤 종료" 흉내
    Gate(Arc<AtomicBool>),
}

/// 명령 -> 각본. `None` 이면 실행 실패(파일 없음)로 흉내 낸다.
pub type Handler = dyn Fn(&CliCommand) -> Option<Vec<Step>> + Send + Sync;

pub struct FakeRunner {
    handler: Box<Handler>,
    next_pid: AtomicU32,
    /// 실행 요청이 들어온 명령 전부 (인자·cwd·stdin 단언용)
    pub spawned: Mutex<Vec<CliCommand>>,
    /// `kill` 이 불린 PID
    pub killed: Arc<Mutex<Vec<u32>>>,
}

impl FakeRunner {
    pub fn new(
        handler: impl Fn(&CliCommand) -> Option<Vec<Step>> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            handler: Box::new(handler),
            next_pid: AtomicU32::new(1000),
            spawned: Mutex::new(Vec::new()),
            killed: Arc::default(),
        })
    }

    pub fn spawn_count(&self) -> usize {
        self.spawned.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// 인자 목록(문자열)에 `arg` 가 포함된 실행 횟수.
    pub fn count_with_arg(&self, arg: &str) -> usize {
        self.spawned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|c| c.args.iter().any(|a| a == arg))
            .count()
    }

    pub fn killed_pids(&self) -> Vec<u32> {
        self.killed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl CliRunner for FakeRunner {
    fn spawn(&self, cmd: &CliCommand) -> io::Result<Box<dyn CliProcess>> {
        self.spawned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(cmd.clone());
        let Some(steps) = (self.handler)(cmd) else {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        Ok(Box::new(FakeProcess {
            pid: self.next_pid.fetch_add(1, Ordering::SeqCst),
            steps: steps.into(),
            killed: Arc::clone(&self.killed),
            dead: false,
        }))
    }

    fn kill_pid(&self, pid: u32) {
        self.killed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(pid);
    }
}

struct FakeProcess {
    pid: u32,
    steps: VecDeque<Step>,
    killed: Arc<Mutex<Vec<u32>>>,
    dead: bool,
}

impl CliProcess for FakeProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn next_event(&mut self, wait: Duration) -> ProcEvent {
        if self.dead {
            return ProcEvent::Idle;
        }
        if let Some(Step::Gate(open)) = self.steps.front()
            && open.load(Ordering::SeqCst)
        {
            self.steps.pop_front();
        }
        match self.steps.front() {
            None | Some(Step::Hang | Step::Gate(_)) => {
                std::thread::sleep(wait.min(Duration::from_millis(20)));
                ProcEvent::Idle
            }
            Some(_) => match self.steps.pop_front() {
                Some(Step::Out(s)) => ProcEvent::Stdout(s),
                Some(Step::Err(s)) => ProcEvent::Stderr(s),
                Some(Step::Exit(c)) => {
                    self.dead = true;
                    ProcEvent::Exit(c)
                }
                _ => ProcEvent::Idle,
            },
        }
    }

    fn kill(&mut self) {
        if !self.dead {
            self.dead = true;
            self.killed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(self.pid);
        }
    }
}
