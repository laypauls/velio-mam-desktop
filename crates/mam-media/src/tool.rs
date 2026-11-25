//! FFmpeg/FFprobe 사이드카 — 경로 탐색, 프로세스 실행(타임아웃·출력 상한·강제 종료).
//!
//! 프로세스 실행은 [`CommandRunner`] trait 뒤에 둔다 — 테스트는 실행기를 주입해 실패 경로를 결정적으로 만든다.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{MediaError, Result};
use crate::gate::SidecarGate;
use crate::proc_tree::ProcessTree;

/// 사이드카 프로세스 기본 제한 시간 (D8).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// ffprobe JSON 출력 상한.
pub const MAX_PROBE_OUTPUT: usize = 4 * 1024 * 1024;
/// 프레임(PNG) 출력 상한.
pub const MAX_FRAME_OUTPUT: usize = 64 * 1024 * 1024;
/// 오류 메시지에 보관하는 stderr 최대 길이.
const STDERR_KEEP: usize = 2048;
/// 프로세스 트리 종료 뒤 파이프 리더를 기다리는 최대 시간 (고아가 파이프를 쥔 폴백 대비).
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// 실행할 외부 도구.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffprobe,
    Ffmpeg,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Ffprobe => "ffprobe",
            Tool::Ffmpeg => "ffmpeg",
        }
    }
}

/// 프로세스 실행 제한.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    pub timeout: Duration,
    pub max_stdout: usize,
}

/// 프로세스 실행 결과. 0 이 아닌 종료 코드도 `Ok` 로 돌려준다 (판정은 호출자).
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    /// 앞부분만 보관한 stderr (lossy UTF-8).
    pub stderr: String,
}

/// 외부 프로세스 실행기. 실제 구현은 [`ProcessRunner`].
pub trait CommandRunner: Send + Sync {
    /// 타임아웃·출력 초과 시 프로세스를 죽이고 `Timeout`/`OutputTooLarge` 를 돌려줘야 한다.
    fn run(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput>;

    /// [`Self::run`] 이되 `cancel()` 이 true 가 되면 실행 중인 프로세스를 죽이고 `Cancelled` 를 돌려준다
    /// (이미 true 면 띄우지도 않는다). 기본 구현은 시작 전에만 확인한다 — 실제 프로세스 실행기는 재정의해
    /// 실행 중에도 확인한다. 긴 작업(장면 검출)의 일시정지·종료 중단용.
    fn run_cancellable(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<CommandOutput> {
        if cancel() {
            return Err(MediaError::Cancelled { tool: tool.name() });
        }
        self.run(tool, program, args, opts)
    }
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
        run_process(tool, program, args, opts, &|| false, &|_| {})
    }

    fn run_cancellable(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<CommandOutput> {
        run_process(tool, program, args, opts, cancel, &|_| {})
    }
}

/// [`ProcessRunner`] 와 같되 `cancel` 이 켜지면 실행 중인 프로세스를 즉시 죽이고 `Cancelled` 를 돌려준다
/// (앱 종료가 사이드카 한 번의 시간 초과를 기다리지 않게 한다). 이미 켜져 있으면 프로세스를 띄우지도 않는다.
#[derive(Debug, Clone)]
pub struct CancellableProcessRunner {
    cancel: Arc<AtomicBool>,
    gate: Option<Arc<SidecarGate>>,
}

impl CancellableProcessRunner {
    pub fn new(cancel: Arc<AtomicBool>) -> Self {
        Self { cancel, gate: None }
    }

    /// 경로 기준 차단·종료 게이트를 연결한다 (USB 꺼내기 준비, `SidecarGate`).
    pub fn with_gate(mut self, gate: Arc<SidecarGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// 게이트에 등록하고 (차단 경로면 `Cancelled`) 실행한다. 게이트가 없으면 종료 신호·호출별 취소만 본다.
    fn run_gated(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<CommandOutput> {
        let Some(gate) = &self.gate else {
            return run_process(
                tool,
                program,
                args,
                opts,
                &|| self.cancel.load(Ordering::SeqCst) || cancel(),
                &|_| {},
            );
        };
        let Some(entry) = gate.enter(args) else {
            return Err(MediaError::Cancelled { tool: tool.name() });
        };
        run_process(
            tool,
            program,
            args,
            opts,
            &|| self.cancel.load(Ordering::SeqCst) || entry.is_cancelled() || cancel(),
            &|pid| entry.set_pid(pid),
        )
    }
}

impl CommandRunner for CancellableProcessRunner {
    fn run(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput> {
        self.run_gated(tool, program, args, opts, &|| false)
    }

    /// 종료 신호 **또는** 호출별 `cancel` 중 하나라도 켜지면 죽인다.
    fn run_cancellable(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<CommandOutput> {
        self.run_gated(tool, program, args, opts, cancel)
    }
}

fn run_process(
    tool: Tool,
    program: &Path,
    args: &[OsString],
    opts: &RunOptions,
    is_cancelled: &dyn Fn() -> bool,
    on_spawn: &dyn Fn(u32),
) -> Result<CommandOutput> {
    if is_cancelled() {
        return Err(MediaError::Cancelled { tool: tool.name() });
    }
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // shim ffmpeg 가 손자를 띄워도 한 번에 죽이도록 Job Object 에 넣는다 (proc_tree.rs).
    let mut tree = ProcessTree::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS (D8: 검색·UI 반응 우선, 사이드카는 낮은 우선순위)
        // CREATE_SUSPENDED: Job 할당 전에 shim 이 손자를 만들지 못하게 멈춘 채 띄운다.
        let suspended = if tree.suspended() { 0x0000_0004 } else { 0 };
        cmd.creation_flags(0x0800_0000 | 0x0000_4000 | suspended);
    }
    let mut child = cmd.spawn().map_err(|e| MediaError::io(program, e))?;
    on_spawn(child.id());
    if let Err(e) = tree.adopt(&child) {
        tree.terminate(&mut child);
        let _ = child.wait();
        return Err(MediaError::io(program, e));
    }

    let exceeded = Arc::new(AtomicBool::new(false));
    let out = child.stdout.take().expect("stdout piped");
    let err = child.stderr.take().expect("stderr piped");
    let (limit, flag) = (opts.max_stdout, Arc::clone(&exceeded));
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = out_tx.send(read_bounded(out, limit, Some(&flag)));
    });
    thread::spawn(move || {
        let _ = err_tx.send(read_bounded(err, STDERR_KEEP, None));
    });

    let started = Instant::now();
    let mut failure: Option<MediaError> = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => {
                failure = Some(MediaError::io(program, e));
                break None;
            }
        }
        if is_cancelled() {
            failure = Some(MediaError::Cancelled { tool: tool.name() });
            break None;
        }
        if exceeded.load(Ordering::Relaxed) {
            failure = Some(MediaError::OutputTooLarge {
                tool: tool.name(),
                limit: opts.max_stdout,
            });
            break None;
        }
        if started.elapsed() >= opts.timeout {
            failure = Some(MediaError::Timeout {
                tool: tool.name(),
                timeout: opts.timeout,
            });
            break None;
        }
        thread::sleep(Duration::from_millis(5));
    };
    // 정상 종료여도 트리를 정리한다 — 남은 손자가 파이프를 쥐고 리더를 막지 못하게.
    tree.terminate(&mut child);
    if status.is_none() {
        // 좀비 방지: 종료시킨 뒤 반드시 wait 로 회수한다.
        let _ = child.wait();
    }
    // 트리를 죽였으니 파이프는 곧 닫힌다. Job 을 못 쓴 폴백에서 고아가 쥐고 있어도 무한 대기하지 않는다.
    let (stdout, out_exceeded) = out_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();
    let (stderr, _) = err_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();
    if let Some(e) = failure {
        return Err(e);
    }
    if out_exceeded {
        return Err(MediaError::OutputTooLarge {
            tool: tool.name(),
            limit: opts.max_stdout,
        });
    }
    Ok(CommandOutput {
        code: status.and_then(|s| s.code()),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// `limit` 바이트까지만 보관한다. `flag` 가 있으면 초과 시 켜고 읽기를 멈춘다(호출자가 프로세스를 죽인다).
/// `flag` 가 없으면 끝까지 읽어 비우되 앞부분만 보관한다(파이프가 막혀 자식이 멈추는 것을 방지).
fn read_bounded(mut r: impl Read, limit: usize, flag: Option<&AtomicBool>) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match r.read(&mut buf) {
            Ok(0) | Err(_) => return (kept, false),
            Ok(n) => {
                let room = limit.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
                if n > room
                    && let Some(flag) = flag
                {
                    flag.store(true, Ordering::Relaxed);
                    return (kept, true);
                }
            }
        }
    }
}

/// 도구 경로 설정. 우선순위: 명시 설정 -> exe 옆 사이드카 -> PATH.
#[derive(Debug, Clone, Default)]
pub struct ToolConfig {
    /// 설정에서 지정한 경로. 실행 파일 자체이거나 그것이 들어 있는 폴더.
    pub ffprobe: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    /// 사이드카 탐색 폴더 (보통 현재 exe 의 폴더).
    pub sidecar_dir: Option<PathBuf>,
    /// PATH 값 (`None` 이면 프로세스 환경의 PATH).
    pub path_env: Option<OsString>,
}

impl ToolConfig {
    /// 현재 exe 옆을 사이드카 폴더로 채운다.
    pub fn with_exe_sidecar(mut self) -> Self {
        self.sidecar_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        self
    }
}

fn exe_name(tool: Tool) -> String {
    if cfg!(windows) {
        format!("{}.exe", tool.name())
    } else {
        tool.name().to_string()
    }
}

/// 도구 경로를 찾는다. **명시 설정이 있는데 잘못됐으면 다른 곳으로 넘어가지 않고 오류**로 돌려준다
/// (PATH 의 다른 빌드가 조용히 쓰이면 LGPL/GPL 배포 구분이 흐려진다).
pub fn resolve_tool(tool: Tool, cfg: &ToolConfig) -> Result<PathBuf> {
    let configured = match tool {
        Tool::Ffprobe => cfg.ffprobe.as_ref(),
        Tool::Ffmpeg => cfg.ffmpeg.as_ref(),
    };
    let name = exe_name(tool);

    if let Some(p) = configured {
        if p.is_file() {
            return Ok(p.clone());
        }
        let inner = p.join(&name);
        if inner.is_file() {
            return Ok(inner);
        }
        return Err(MediaError::ToolNotFound {
            tool: tool.name(),
            searched: format!("configured {}", p.display()),
        });
    }
    let mut searched: Vec<String> = Vec::new();
    if let Some(dir) = &cfg.sidecar_dir {
        searched.push(format!("sidecar {}", dir.display()));
        let c = dir.join(&name);
        if c.is_file() {
            return Ok(c);
        }
    }
    let path_env = cfg
        .path_env
        .clone()
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    searched.push("PATH".to_string());
    for dir in std::env::split_paths(&path_env) {
        let c = dir.join(&name);
        if c.is_file() {
            return Ok(c);
        }
    }
    Err(MediaError::ToolNotFound {
        tool: tool.name(),
        searched: searched.join(", "),
    })
}

type Resolved = std::result::Result<PathBuf, Arc<str>>;

/// 해석된 도구 경로 + 실행기 + 제한. 도구가 없으면 해당 메서드만 `ToolNotFound` 로 실패한다.
#[derive(Clone)]
pub struct MediaTools {
    ffprobe: Resolved,
    ffmpeg: Resolved,
    runner: Arc<dyn CommandRunner>,
    pub timeout: Duration,
    /// ffmpeg `-threads` 값 (사용자 PC 리소스 절약, D8).
    pub threads: u32,
}

impl MediaTools {
    /// 경로를 탐색한다. 못 찾은 도구는 기능 저하 상태로 두고 실패하지 않는다.
    pub fn discover(cfg: &ToolConfig, runner: Arc<dyn CommandRunner>) -> Self {
        let r = |t| resolve_tool(t, cfg).map_err(|e| Arc::<str>::from(e.to_string()));
        Self {
            ffprobe: r(Tool::Ffprobe),
            ffmpeg: r(Tool::Ffmpeg),
            runner,
            timeout: DEFAULT_TIMEOUT,
            threads: 2,
        }
    }

    /// 경로를 직접 지정한다 (테스트용 스텁 실행기 주입에 쓴다).
    pub fn with_paths(
        ffprobe: Option<PathBuf>,
        ffmpeg: Option<PathBuf>,
        runner: Arc<dyn CommandRunner>,
    ) -> Self {
        let miss = |t: Tool| Arc::<str>::from(format!("{} not configured", t.name()));
        Self {
            ffprobe: ffprobe.ok_or_else(|| miss(Tool::Ffprobe)),
            ffmpeg: ffmpeg.ok_or_else(|| miss(Tool::Ffmpeg)),
            runner,
            timeout: DEFAULT_TIMEOUT,
            threads: 2,
        }
    }

    pub fn has_ffprobe(&self) -> bool {
        self.ffprobe.is_ok()
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg.is_ok()
    }

    fn program(&self, tool: Tool) -> Result<&PathBuf> {
        match tool {
            Tool::Ffprobe => &self.ffprobe,
            Tool::Ffmpeg => &self.ffmpeg,
        }
        .as_ref()
        .map_err(|why| MediaError::ToolNotFound {
            tool: tool.name(),
            searched: why.to_string(),
        })
    }

    /// 도구를 실행하고 0 종료 코드만 성공으로 본다.
    pub(crate) fn run(&self, tool: Tool, args: &[OsString], max_stdout: usize) -> Result<Vec<u8>> {
        let program = self.program(tool)?;
        let opts = RunOptions {
            timeout: self.timeout,
            max_stdout,
        };
        let out = self.runner.run(tool, program, args, &opts)?;
        Self::check_exit(tool, out)
    }

    /// [`Self::run`] 이되 호출별 제한(`opts`)을 쓰고 `cancel()` 이 켜지면 프로세스를 죽인다 (`Cancelled`).
    /// 전역 `timeout` 은 쓰지 않는다 — 긴 작업이 호출 단위로 시간 제한을 정한다.
    pub fn run_with(
        &self,
        tool: Tool,
        args: &[OsString],
        opts: RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>> {
        let program = self.program(tool)?;
        let out = self
            .runner
            .run_cancellable(tool, program, args, &opts, cancel)?;
        Self::check_exit(tool, out)
    }

    fn check_exit(tool: Tool, out: CommandOutput) -> Result<Vec<u8>> {
        if out.code != Some(0) {
            return Err(MediaError::ToolFailed {
                tool: tool.name(),
                code: out.code,
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(out.stdout)
    }
}
