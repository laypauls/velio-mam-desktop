//! 외부 도구(whisper-cli, ffmpeg) 실행 - 경로 탐색, 타임아웃·취소·출력 상한, 강제 종료 후 회수(좀비 방지).
//!
//! `mam-media` 의 `tool.rs` 와 같은 방식이다(`CommandRunner` trait 로 실행기를 주입해 시험에서 실패를 결정적으로
//! 만든다). 두 crate 가 서로 의존하지 않도록 필요한 부분만 복제했다 - 통합은 P3 에서 판단한다.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::proc_tree::ProcessTree;
use crate::error::{AiError, Result};

/// 오류 메시지에 보관하는 stderr 최대 길이.
const STDERR_KEEP: usize = 2048;
/// stdout 은 앞부분만 보관한다 (whisper 는 전사문을 stdout 에도 찍는다 - 결과는 JSON 파일에서 읽는다).
const STDOUT_KEEP: usize = 64 * 1024;
/// 프로세스 트리 종료 뒤 파이프 리더를 기다리는 최대 시간.
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    WhisperCli,
    Ffmpeg,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::WhisperCli => "whisper-cli",
            Tool::Ffmpeg => "ffmpeg",
        }
    }
}

/// 취소 신호 - 복제해 다른 스레드에서 `cancel()` 하면 실행 중인 프로세스를 죽인다.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub timeout: Duration,
    pub cancel: Option<CancelToken>,
    /// 사용자 PC 반응을 우선하도록 낮은 우선순위(BELOW_NORMAL)로 실행한다.
    pub low_priority: bool,
}

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub code: Option<i32>,
    /// 앞부분만 보관한 stdout (lossy UTF-8).
    pub stdout: String,
    /// 앞부분만 보관한 stderr (lossy UTF-8).
    pub stderr: String,
}

/// 외부 프로세스 실행기. 시간 초과·취소 시 프로세스를 죽이고 `Timeout`/`Cancelled` 를 돌려줘야 한다.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput>;
}

/// `std::process` 기반 실행기.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // shim/wrapper 가 손자를 띄워도 한 번에 죽이도록 Job Object 에 넣는다 (proc_tree.rs).
        let mut tree = ProcessTree::new();
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            const CREATE_SUSPENDED: u32 = 0x0000_0004;
            let mut flags = CREATE_NO_WINDOW;
            if opts.low_priority {
                flags |= BELOW_NORMAL_PRIORITY_CLASS;
            }
            if tree.suspended() {
                flags |= CREATE_SUSPENDED; // Job 할당 전에 손자가 생기지 않게
            }
            cmd.creation_flags(flags);
        }
        let mut child = cmd.spawn().map_err(|e| AiError::io(program, e))?;
        if let Err(e) = tree.adopt(&child) {
            tree.terminate(&mut child);
            let _ = child.wait();
            return Err(AiError::io(program, e));
        }
        let out = child.stdout.take().expect("stdout piped");
        let err = child.stderr.take().expect("stderr piped");
        let (out_tx, out_rx) = mpsc::channel();
        let (err_tx, err_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = out_tx.send(read_bounded(out, STDOUT_KEEP));
        });
        thread::spawn(move || {
            let _ = err_tx.send(read_bounded(err, STDERR_KEEP));
        });

        let started = Instant::now();
        let mut failure: Option<AiError> = None;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(e) => {
                    failure = Some(AiError::io(program, e));
                    break None;
                }
            }
            if opts.cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
                failure = Some(AiError::Cancelled { tool: tool.name() });
                break None;
            }
            if started.elapsed() >= opts.timeout {
                failure = Some(AiError::Timeout {
                    tool: tool.name(),
                    timeout: opts.timeout,
                });
                break None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        // 정상 종료여도 트리를 정리한다 - 남은 손자가 파이프를 쥐고 리더를 막지 못하게.
        tree.terminate(&mut child);
        if status.is_none() {
            // 좀비 방지: 종료시킨 뒤 반드시 wait 로 회수한다.
            let _ = child.wait();
        }
        // Job 을 못 쓴 폴백에서 고아가 파이프를 쥐고 있어도 무한 대기하지 않는다.
        let stdout = out_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();
        let stderr = err_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();
        if let Some(e) = failure {
            return Err(e);
        }
        Ok(CommandOutput {
            code: status.and_then(|s| s.code()),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
}

/// 끝까지 읽어 파이프를 비우되(자식이 막히지 않게) 앞 `limit` 바이트만 보관한다.
fn read_bounded(mut r: impl Read, limit: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => return kept,
            Ok(n) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
}

fn exe_name(tool: Tool) -> String {
    if cfg!(windows) {
        format!("{}.exe", tool.name())
    } else {
        tool.name().to_string()
    }
}

/// 도구 경로를 찾는다. 우선순위: 명시 설정 -> 사이드카 폴더(exe 옆) -> PATH.
/// **명시 설정이 있는데 잘못됐으면 다른 곳으로 넘어가지 않고 오류**로 돌려준다 (PATH 의 다른 빌드가 조용히 쓰이지 않게).
pub fn resolve_tool(
    tool: Tool,
    configured: Option<&Path>,
    sidecar_dir: Option<&Path>,
    path_env: Option<&OsString>,
) -> Result<PathBuf> {
    let name = exe_name(tool);
    if let Some(p) = configured {
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        let inner = p.join(&name);
        if inner.is_file() {
            return Ok(inner);
        }
        return Err(AiError::ToolNotFound {
            tool: tool.name(),
            searched: format!("configured {}", p.display()),
        });
    }
    let mut searched = Vec::new();
    if let Some(dir) = sidecar_dir {
        searched.push(format!("sidecar {}", dir.display()));
        let c = dir.join(&name);
        if c.is_file() {
            return Ok(c);
        }
    }
    let path_env = path_env
        .cloned()
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    searched.push("PATH".to_string());
    for dir in std::env::split_paths(&path_env) {
        let c = dir.join(&name);
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(AiError::ToolNotFound {
        tool: tool.name(),
        searched: searched.join(", "),
    })
}
