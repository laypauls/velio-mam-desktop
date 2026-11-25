//! `CliHost` — 설치된 CLI 와 대화하는 모든 경로의 공용 소유자: 탐지(경로·버전·로그인 상태), 요청 직렬화, 로그아웃, 종료 정리.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use super::exec::{Exec, RunEnd};
use super::process::{CliCommand, CliRunner};
use super::resolve::{CliEnv, resolve_claude};
use super::status::{CliStatus, MIN_VERSION, parse_auth_status, parse_version};
use super::workdir;
use crate::external::error::ExternalError;
use crate::external::retry::Clock;

/// `--version`·`auth status` 제한 시간 (네트워크 없이 끝나는 명령 — 걸리면 응답 없는 CLI 로 본다).
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// `auth logout` 제한 시간.
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct CliHost {
    pub(super) exec: Exec,
    pub(super) env: Arc<dyn CliEnv>,
    pub(super) clock: Arc<dyn Clock>,
    /// 요청 작업 폴더의 부모 (운영: `<OS temp>/mam-ai-cli` — `workdir::default_work_root`)
    pub(super) work_root: PathBuf,
    /// 구독 요청은 프로세스 단위로 한 번에 하나 (구독 속도 제한·약관의 개인 사용 가정)
    serial: Mutex<()>,
    /// 버전 확인을 통과한 실행 파일 (같은 경로면 다시 확인하지 않는다)
    verified: Mutex<Option<PathBuf>>,
    /// `--version`·`auth status` 제한 시간(ms) — 시험이 줄일 수 있다
    probe_timeout_ms: AtomicU64,
    /// 구독 한도에 걸려 요청을 보내면 안 되는 시각(유닉스 ms). 메모리에만 둔다 — 재시작 뒤에는 작업 대기열의 `not_before` 가 이어받는다
    paused_until: Mutex<Option<i64>>,
}

impl CliHost {
    /// 시작 시 이전 실행이 남긴 작업 폴더를 정리한다.
    pub fn new(
        runner: Arc<dyn CliRunner>,
        env: Arc<dyn CliEnv>,
        clock: Arc<dyn Clock>,
        work_root: PathBuf,
    ) -> Arc<Self> {
        workdir::sweep(&work_root);
        Arc::new(Self {
            exec: Exec::new(runner),
            env,
            clock,
            work_root,
            serial: Mutex::new(()),
            verified: Mutex::new(None),
            probe_timeout_ms: AtomicU64::new(PROBE_TIMEOUT.as_millis() as u64),
            paused_until: Mutex::new(None),
        })
    }

    /// 시험용: 탐지 명령 제한 시간을 줄인다.
    pub fn set_probe_timeout(&self, d: Duration) {
        let ms = u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        self.probe_timeout_ms.store(ms, Ordering::SeqCst);
    }

    fn probe_timeout(&self) -> Duration {
        Duration::from_millis(self.probe_timeout_ms.load(Ordering::SeqCst))
    }

    /// 한도 해제 시각 (아직 그 전일 때만). 지난 값은 지운다.
    pub fn paused_until_ms(&self) -> Option<i64> {
        let mut g = self.paused_until.lock().unwrap_or_else(|e| e.into_inner());
        match *g {
            Some(t) if t > self.clock.now_ms() => Some(t),
            _ => {
                *g = None;
                None
            }
        }
    }

    /// 지금부터 `secs` 초 동안 구독 요청을 보내지 않는다 (이미 더 늦은 해제 시각이 있으면 유지).
    pub fn pause_for_secs(&self, secs: u64) {
        let add = i64::try_from(secs)
            .unwrap_or(i64::MAX / 2000)
            .saturating_mul(1000);
        let until = self.clock.now_ms().saturating_add(add);
        let mut g = self.paused_until.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(g.map_or(until, |old| old.max(until)));
    }

    pub fn clear_pause(&self) {
        *self.paused_until.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(super) fn exec(&self) -> &Exec {
        &self.exec
    }

    pub(super) fn serial(&self) -> MutexGuard<'_, ()> {
        self.serial.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 환경변수 API 키 존재 여부 (값은 읽지 않는다).
    pub fn api_key_env(&self) -> bool {
        self.env.has_var("ANTHROPIC_API_KEY") || self.env.has_var("ANTHROPIC_AUTH_TOKEN")
    }

    pub fn resolve(&self) -> Option<PathBuf> {
        resolve_claude(self.env.as_ref())
    }

    fn command(&self, exe: PathBuf, args: &[&str]) -> CliCommand {
        CliCommand {
            program: exe,
            args: args.iter().map(OsString::from).collect(),
            cwd: None,
            stdin: None,
        }
    }

    /// 요청 직전 확인: 찾을 수 있고 버전이 최소 이상이어야 한다. 통과한 경로는 기억한다.
    pub fn ensure_ready(&self) -> Result<PathBuf, ExternalError> {
        let exe = self.resolve().ok_or(ExternalError::CliNotFound)?;
        if self
            .verified
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            == Some(&exe)
        {
            return Ok(exe);
        }
        let ver = self.probe_version(&exe)?;
        if ver.is_none_or(|v| v < MIN_VERSION) {
            return Err(ExternalError::CliUnsupportedVersion);
        }
        *self.verified.lock().unwrap_or_else(|e| e.into_inner()) = Some(exe.clone());
        Ok(exe)
    }

    /// `--version` 실행. 출력을 해석할 수 없으면 `Ok(None)`(지원 안 함). 일시적 실패를 영구 코드로 바꾸지 않는다:
    /// 취소·종료 -> `Cancelled`, 제한 시간 -> `Timeout`(재시도 대상), 실행 불가 -> `CliNotFound`.
    fn probe_version(&self, exe: &Path) -> Result<Option<super::status::Version>, ExternalError> {
        let out = self
            .exec
            .capture(
                &self.command(exe.to_path_buf(), &["--version"]),
                self.probe_timeout(),
            )
            .map_err(|_| {
                if self.exec.is_cancelled() {
                    ExternalError::Cancelled
                } else {
                    ExternalError::CliNotFound
                }
            })?;
        match out.end {
            RunEnd::Exited(Some(0)) => Ok(parse_version(&out.stdout)),
            RunEnd::Cancelled => Err(ExternalError::Cancelled),
            RunEnd::Timeout => Err(ExternalError::Timeout),
            _ => Ok(None),
        }
    }

    /// 설치·버전·로그인 상태 재확인. 네트워크 요청을 하지 않는 명령만 쓴다(동의 전에도 허용).
    /// `auth status` 실패는 `account: None`(알 수 없음)이지 로그아웃이 아니다.
    pub fn detect(&self) -> CliStatus {
        let api_key_env = self.api_key_env();
        let Some(exe) = self.resolve() else {
            return CliStatus {
                found: false,
                version: None,
                supported: false,
                account: None,
                api_key_env,
            };
        };
        // 제한 시간·취소 같은 일시적 실패는 판정이 아니다 - 확인된 경로를 그대로 둔다
        let probed = self.probe_version(&exe);
        let ver = probed.as_ref().ok().copied().flatten();
        let supported = ver.is_some_and(|v| v >= MIN_VERSION);
        let account = if supported {
            self.read_account(&exe)
        } else {
            None
        };
        if probed.is_ok() {
            let mut v = self.verified.lock().unwrap_or_else(|e| e.into_inner());
            if !supported {
                *v = None;
            } else if v.as_ref() != Some(&exe) {
                *v = Some(exe);
            }
        }
        CliStatus {
            found: true,
            version: ver.map(|v| v.to_string()),
            supported,
            account,
            api_key_env,
        }
    }

    /// `auth status --json` (로그인 완료 판정에도 쓴다). 실패 -> `None`.
    pub(super) fn read_account(&self, exe: &Path) -> Option<super::status::CliAccount> {
        let out = self
            .exec
            .capture(
                &self.command(exe.to_path_buf(), &["auth", "status", "--json"]),
                self.probe_timeout(),
            )
            .ok()?;
        // 종료 코드는 보지 않는다 — 로그아웃 상태도 `loggedIn:false` 를 출력한다
        if !matches!(out.end, RunEnd::Exited(_)) {
            return None;
        }
        parse_auth_status(&out.stdout)
    }

    /// `claude auth logout`. 실행 중인 구독 요청이 있으면(직렬화 잠금을 못 잡으면) `Busy`.
    pub fn logout(&self) -> Result<(), LogoutError> {
        let _serial = match self.serial.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(LogoutError::Busy),
        };
        let exe = self.resolve().ok_or(LogoutError::NotFound)?;
        let out = self
            .exec
            .capture(&self.command(exe, &["auth", "logout"]), LOGOUT_TIMEOUT)
            .map_err(|_| LogoutError::Failed)?;
        match out.end {
            RunEnd::Exited(Some(0)) => Ok(()),
            _ => Err(LogoutError::Failed),
        }
    }

    /// 종료 경로: 실행 중인 모든 CLI 프로세스(요청·로그인)를 PID 로 종료하고 이후 실행을 막는다.
    pub fn kill_all(&self) {
        self.exec.kill_all();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoutError {
    /// 진행 중인 구독 요청이 있다
    Busy,
    NotFound,
    Failed,
}
