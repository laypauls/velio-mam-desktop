//! 프로세스 실행 추상화 — 시험에서 실제 `claude` 를 띄우지 않도록 실행기를 trait 로 주입한다.
//!
//! 규약: 셸을 거치지 않는다(인자는 argv 로만), 환경은 상속한다(인증 방식을 제한하지 않는다 — 약관),
//! 종료는 **PID 로만** 한다(이름으로 죽이지 않는다).

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// 실행할 명령. `stdin` 이 `Some` 이면 전부 쓰고 닫는다. `None` 이면 stdin 은 닫힌 상태(null)다.
#[derive(Debug, Clone)]
pub struct CliCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// MAM 이 만든 빈 임시 폴더. 원본 미디어 경로를 넣지 않는다.
    pub cwd: Option<PathBuf>,
    pub stdin: Option<Vec<u8>>,
}

/// 자식 프로세스에서 온 사건. 출력은 UTF-8 경계가 보정된 조각이다(줄 단위가 아닐 수 있다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcEvent {
    Stdout(String),
    Stderr(String),
    /// 종료 (출력은 이미 모두 전달된 뒤)
    Exit(Option<i32>),
    /// 대기 시간 동안 아무 일도 없었다
    Idle,
}

pub trait CliProcess: Send {
    fn pid(&self) -> u32;
    /// 최대 `wait` 동안 다음 사건을 기다린다.
    fn next_event(&mut self, wait: Duration) -> ProcEvent;
    /// 프로세스(트리)를 강제 종료하고 회수한다. 여러 번 불러도 안전하다.
    fn kill(&mut self);
}

pub trait CliRunner: Send + Sync {
    fn spawn(&self, cmd: &CliCommand) -> io::Result<Box<dyn CliProcess>>;
    /// PID 로 프로세스 트리를 종료한다 (종료 경로가 소유자 스레드 없이 부른다). 기본은 아무것도 하지 않는다.
    fn kill_pid(&self, _pid: u32) {}
}
