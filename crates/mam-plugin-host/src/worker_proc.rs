//! 워커 프로세스 1개 — 실행·쓰기·종료. 종료는 **자기가 띄운 핸들(PID)로만** 한다 (이미지 이름으로 죽이지 않는다).

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, sync_channel};

use crate::job::{KillOnClose, process_memory_cap};
use crate::line_reader::{LineEvent, pump_lines};
use crate::protocol::{HostMsg, encode_line};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// 읽기 스레드 -> 감독자 채널 크기. 가득 차면 읽기 스레드가 막히고 워커 쓰기가 파이프에서 멈춘다(배압)
const CHANNEL_CAP: usize = 256;

#[derive(Default)]
pub(crate) struct Handlers {
    pub commands: HashSet<String>,
    pub providers: HashSet<String>,
}

/// 워커에 사용자 환경(API 키 등 비밀 환경변수)·앱 작업 폴더를 물려주지 않는다.
/// 환경은 비우고 Windows 가 DLL 을 찾는 데 필요한 `SystemRoot` 와 빈 샌드박스 폴더를 가리키는 `TEMP`/`TMP` 만 준다. 작업 폴더도 그 빈 폴더다
/// (MAMData·관리 폴더가 아니다). 워커는 `MAM_*` 등 어떤 환경변수도 읽지 않는다. 표준 입출력 외 상속 핸들은 std 가 만들지 않는다
fn harden(cmd: &mut Command) -> std::io::Result<()> {
    let dir = sandbox_dir()?;
    cmd.env_clear().current_dir(&dir).env("TEMP", &dir).env("TMP", &dir);
    if let Some(root) = std::env::var_os("SystemRoot") {
        cmd.env("SystemRoot", root);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    Ok(())
}

/// 워커 전용 빈 폴더 — OS 임시 폴더 아래 고정 이름. 워커는 파일을 열지 않으므로 항상 비어 있다
fn sandbox_dir() -> std::io::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("mam-plugin-worker-sandbox");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub(crate) struct WorkerProc {
    child: Child,
    /// 호스트 프로세스가 죽으면 워커도 죽게 하는 Job (할당 실패 시 `None` — 평소 종료 경로만 보호)
    _job: Option<KillOnClose>,
    stdin: ChildStdin,
    pub rx: Receiver<LineEvent>,
    pub handlers: Handlers,
}

impl WorkerProc {
    pub fn spawn(exe: &Path, max_line: usize, quickjs_memory_bytes: u64) -> std::io::Result<Self> {
        // 경로가 없으면(워커를 못 찾음) 아무것도 띄우지 않는다 — 빈 경로가 PATH·작업 폴더 탐색으로 번지지 않게 한다
        if exe.as_os_str().is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "worker executable not configured"));
        }
        let mut cmd = Command::new(exe);
        harden(&mut cmd)?;
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        // 띄운 직후 Job 에 넣는다 — 호스트가 크래시해도 워커가 고아로 남지 않는다
        let job = KillOnClose::attach(&child, process_memory_cap(quickjs_memory_bytes));
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other("worker pipes unavailable"));
        };
        let (tx, rx) = sync_channel(CHANNEL_CAP);
        // 파이프가 닫히면(워커 종료·kill) 읽기가 끝나 스레드도 끝난다
        std::thread::Builder::new().name("mam-plugin-reader".into()).spawn(move || pump_lines(stdout, max_line, &tx))?;
        Ok(Self { child, _job: job, stdin, rx, handlers: Handlers::default() })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn has_exited(&mut self) -> bool {
        self.child.try_wait().map(|s| s.is_some()).unwrap_or(true)
    }

    pub fn send(&mut self, msg: &HostMsg) -> std::io::Result<()> {
        let line = encode_line(msg).map_err(std::io::Error::other)?;
        self.stdin.write_all(&line)?;
        self.stdin.flush()
    }

    /// 이 핸들의 프로세스만 종료하고 회수한다. 이미 끝났으면 무해
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for WorkerProc {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn run_hardened(args: &[&str]) -> String {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(args);
        harden(&mut cmd).expect("harden");
        let out = cmd.output().expect("run cmd");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// T-phost-env-01 (Business rule): 호스트의 환경변수(PATH·USERPROFILE·APPDATA 등)가 워커에 보이지 않고 SystemRoot·TEMP·TMP 만 있다
    #[test]
    fn t_phost_env_01_environment_is_cleared_except_the_allowlist() {
        // Arrange: 호스트 환경에는 사용자 변수가 있다 (비밀 키 이름 포함 가능)
        assert!(std::env::var_os("USERPROFILE").is_some() && std::env::var_os("PATH").is_some());
        // Act
        let dump = run_hardened(&["/D", "/C", "set"]);
        // Assert
        let mut names: Vec<String> = dump.lines().filter_map(|l| l.split_once('=').map(|(k, _)| k.to_ascii_uppercase())).collect();
        names.sort();
        assert!(names.iter().all(|n| ["SYSTEMROOT", "TEMP", "TMP", "PROMPT", "PATHEXT", "COMSPEC"].contains(&n.as_str()) || n.starts_with('=')), "unexpected env: {names:?}");
        assert!(!names.iter().any(|n| ["USERPROFILE", "PATH", "APPDATA", "OPENAI_API_KEY"].contains(&n.as_str())), "leaked: {names:?}");
        assert!(names.contains(&"SYSTEMROOT".to_owned()));
    }

    /// T-phost-env-02 (Business rule): 작업 폴더는 앱 cwd 가 아니라 비어 있는 샌드박스 폴더다
    #[test]
    fn t_phost_env_02_cwd_is_the_empty_sandbox_dir_not_the_app_cwd() {
        let dump = run_hardened(&["/D", "/C", "cd"]);
        let want = sandbox_dir().expect("dir");
        assert_eq!(std::path::Path::new(dump.trim()).canonicalize().expect("cwd"), want.canonicalize().expect("want"));
        assert_ne!(std::path::Path::new(dump.trim()).canonicalize().expect("cwd"), std::env::current_dir().expect("app cwd").canonicalize().expect("c"));
        assert_eq!(std::fs::read_dir(&want).expect("read").count(), 0, "sandbox dir must stay empty");
    }
}
