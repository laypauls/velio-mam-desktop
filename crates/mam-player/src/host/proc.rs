//! 워커 프로세스 실행·감시. 종료 감시는 프로세스 핸들 대기이고, 종료는 **자기가 띄운 핸들로만** 한다 (이미지 이름으로 죽이지 않는다).
//! `WorkerSpawner` 뒤에 둬서 감독자(supervisor)를 가짜 워커로도 시험할 수 있다.

use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::WaitForSingleObject;

use super::pipe;
use crate::PlayerError;

/// 프로세스 핸들 값 (`DuplicateHandle` 대상 지정용 — 자식 프로세스 핸들)
pub type RawProcessHandle = isize;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// 워커가 파이프를 만들 때까지 기다리는 상한
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub trait WorkerProcess: Send {
    fn id(&self) -> u32;
    fn raw_handle(&self) -> RawProcessHandle;
    fn has_exited(&mut self) -> bool;
    /// 이 핸들의 프로세스만 종료하고 회수한다. 이미 끝났으면 아무 일도 하지 않는다
    fn kill(&mut self);
    /// 종료를 `timeout` 까지 기다린다. 끝났으면 true
    fn wait_exit(&mut self, timeout: Duration) -> bool;
}

/// 워커와의 연결 — 명령 쓰기·이벤트 읽기 스트림과 프로세스 핸들
pub struct WorkerConn {
    pub process: Box<dyn WorkerProcess>,
    pub cmd: Box<dyn Write + Send>,
    pub evt: Box<dyn Read + Send>,
}

pub trait WorkerSpawner: Send + Sync {
    /// 워커를 띄우고 연결한다 (감독자 스레드에서 호출 — 수 초 걸릴 수 있다)
    fn spawn(&self) -> Result<WorkerConn, PlayerError>;
}

/// 실제 자식 프로세스
pub struct ChildProcess {
    child: Child,
}

impl WorkerProcess for ChildProcess {
    fn id(&self) -> u32 {
        self.child.id()
    }

    fn raw_handle(&self) -> RawProcessHandle {
        self.child.as_raw_handle() as isize
    }

    fn has_exited(&mut self) -> bool {
        self.child.try_wait().map(|s| s.is_some()).unwrap_or(true)
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn wait_exit(&mut self, timeout: Duration) -> bool {
        if self.has_exited() {
            return true;
        }
        let ms = timeout.as_millis().min(u32::MAX as u128 - 1) as u32;
        // SAFETY: child 가 살아 있는 동안 유효한 프로세스 핸들
        let r = unsafe { WaitForSingleObject(HANDLE(self.child.as_raw_handle()), ms) };
        r == WAIT_OBJECT_0 && self.has_exited()
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        if !self.has_exited() {
            self.kill();
        }
    }
}

/// `mam-player-worker.exe` 를 띄우는 기본 구현
pub struct ProcessSpawner {
    pub worker_exe: PathBuf,
    pub libmpv_dir: PathBuf,
    /// 워커 stderr 를 호스트 stderr 로 내보낸다 (`MAM_PLAYER_LOG` 환경변수) — 기본은 버린다
    pub inherit_stderr: bool,
    /// 워커에 추가로 줄 환경변수 (시험: `MAM_PLAYER_AO=null` 로 소리를 끈다). 제품은 비운다
    pub env: Vec<(String, String)>,
}

impl WorkerSpawner for ProcessSpawner {
    fn spawn(&self) -> Result<WorkerConn, PlayerError> {
        let io = |what: &str, e: std::io::Error| PlayerError::SpawnFailed(format!("{what}: {e}"));
        let name = pipe::random_pipe_name().map_err(|e| io("pipe name", e))?;
        let child = std::process::Command::new(&self.worker_exe)
            .arg("--pipe")
            .arg(&name)
            .arg("--mpv")
            .arg(&self.libmpv_dir)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(if self.inherit_stderr { Stdio::inherit() } else { Stdio::null() })
            .spawn()
            .map_err(|e| io("spawn", e))?;
        let mut process = ChildProcess { child };
        // 워커가 파이프를 만들 때까지 재시도 — 워커가 먼저 죽었는지 확인하며 상한 안에서만
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match pipe::open_client(&name) {
                Ok((cmd, evt)) => return Ok(WorkerConn { process: Box::new(process), cmd: Box::new(cmd), evt: Box::new(evt) }),
                Err(e) => {
                    if process.has_exited() {
                        return Err(PlayerError::SpawnFailed("worker exited before connecting".into()));
                    }
                    if Instant::now() >= deadline {
                        return Err(PlayerError::SpawnFailed(format!("pipe connect timed out: {e}")));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: External(비정상 종료) / Validation(실행 파일 없음)

    fn spawn_cmd(args: &[&str]) -> ChildProcess {
        ChildProcess { child: std::process::Command::new("cmd").args(args).stdout(Stdio::null()).spawn().unwrap() }
    }

    #[test]
    fn t_player_exit_01_process_exit_is_detected() {
        let mut p = spawn_cmd(&["/C", "exit 7"]);
        assert!(p.wait_exit(Duration::from_secs(10)));
        assert!(p.has_exited());
    }

    #[test]
    fn t_player_exit_02_running_process_times_out_then_kill_by_handle() {
        let child = std::process::Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        let mut p = ChildProcess { child };
        assert!(!p.wait_exit(Duration::from_millis(100)), "아직 실행 중");
        p.kill();
        assert!(p.has_exited());
        p.kill(); // 두 번째 kill 은 무해
    }

    #[test]
    fn t_player_exit_03_spawn_missing_exe_is_error() {
        let s = ProcessSpawner { worker_exe: PathBuf::from(r"C:\no\such\worker.exe"), libmpv_dir: PathBuf::from("."), inherit_stderr: false, env: vec![] };
        assert!(matches!(s.spawn(), Err(PlayerError::SpawnFailed(_))));
    }

    #[test]
    fn t_player_exit_04_worker_dying_before_connect_is_reported_quickly() {
        // 파이프를 만들지 않고 바로 끝나는 프로그램 → 연결 대기 중 종료를 감지해 10초를 다 기다리지 않는다
        let s = ProcessSpawner { worker_exe: PathBuf::from(r"C:\Windows\System32\whoami.exe"), libmpv_dir: PathBuf::from("."), inherit_stderr: false, env: vec![] };
        let t = Instant::now();
        let r = s.spawn();
        assert!(matches!(r, Err(PlayerError::SpawnFailed(_))));
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }
}
