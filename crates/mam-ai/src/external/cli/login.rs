//! 브라우저 로그인 연동 — `claude auth login --claudeai` 를 실행해 출력의 OAuth URL 을 시스템 기본 브라우저로 연다.
//!
//! - MAM 은 로그인 창·입력 폼을 만들지 않는다. 사용자가 브라우저에서 승인하면 CLI 가 스스로 종료하고, 판정은 `auth status`
//! - 브라우저로 여는 URL 은 **CLI 출력에서 잡은 `https://` 인증 URL 뿐**이다 (UI 가 URL 을 지정할 수 없다)
//! - **코드 중계 없음** — 브라우저가 코드를 보여 주는 드문 경우는 UI 가 터미널 안내로 대체한다 (stdin 은 닫혀 있다, 약관)
//! - 단일 세션(single-flight): `start` 는 처음부터 끝까지 시작 잠금을 쥔다. 다시 시작하면 이전 세션의 프로세스를 PID 로 종료한다
//! - 취소·로그아웃·인증 방식 전환(`cancel`)은 세대 번호를 올린다. 시작이 URL 을 기다리는 사이 세대가 바뀌었으면
//!   새 세션을 저장하지 않고 즉시 종료한다 (취소가 새로 뜬 프로세스를 놓치지 않는다). 세션을 놓으면(`Drop`) 프로세스도 종료된다
//! - 세션 전체 제한 시간(10분)이 지나면 프로세스를 종료하고 `failed` 로 알린다
//! - 상태 명령이 실패하면 "알 수 없음"으로 처리하고 로그아웃으로 단정하지 않는다

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::host::CliHost;
use super::login_url::{last_authorize_url, normalize_output};
use super::process::{CliCommand, ProcEvent};
use super::status::CliAccount;
use super::workdir::WorkDir;

/// URL 이 출력될 때까지 기다리는 시간.
const URL_TIMEOUT: Duration = Duration::from_secs(20);
/// 로그인 세션 전체 제한 시간 — 사용자가 브라우저에서 승인하지 않고 방치한 CLI 를 남기지 않는다.
const SESSION_TIMEOUT: Duration = Duration::from_secs(600);
/// 첫 URL 조각을 잡은 뒤 이어지는 출력(줄바꿈된 나머지)을 기다리는 시간.
const URL_JOIN: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(50);
/// 보관하는 출력 끝부분 길이(바이트) — URL 추출 전용, 로그·UI 로 내보내지 않는다.
const OUT_KEEP: usize = 16 * 1024;

/// 기본 브라우저로 URL 을 연다. Windows 구현은 앱(`src-tauri`)이 `ShellExecute` 로 제공한다 — 프론트에는 노출하지 않는다.
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> io::Result<()>;
}

/// UI `AiCliLoginState`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum LoginState {
    None,
    Waiting {
        url: Option<String>,
    },
    Done {
        verified: bool,
    },
    Failed {
        #[serde(rename = "errorCode")]
        error_code: String,
    },
}

impl LoginState {
    fn failed(code: &str) -> Self {
        Self::Failed {
            error_code: code.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoginTimings {
    pub url_timeout: Duration,
    pub url_join: Duration,
    pub session_timeout: Duration,
}

impl Default for LoginTimings {
    fn default() -> Self {
        Self {
            url_timeout: URL_TIMEOUT,
            url_join: URL_JOIN,
            session_timeout: SESSION_TIMEOUT,
        }
    }
}

#[derive(Default)]
struct Phase {
    out: String,
    url: Option<String>,
    /// 자식이 끝났고 판정이 나왔다 (한 번만 전달)
    finished: Option<LoginState>,
}

#[derive(Default)]
struct Shared {
    phase: Mutex<Phase>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Session {
    shared: Arc<Shared>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    /// 프로세스를 종료시키고 감시 스레드가 끝나길 기다린다 (`Drop` 이 한다).
    fn stop(self) {
        drop(self);
    }
}

impl Drop for Session {
    /// 어떤 경로로 놓여도 프로세스가 남지 않는다 (감시 스레드는 `POLL` 안에 취소를 보고 PID 로 종료한다).
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub struct CliLogin {
    host: Arc<CliHost>,
    opener: Arc<dyn UrlOpener>,
    timings: LoginTimings,
    session: Mutex<Option<Session>>,
    /// `start` 전체를 직렬화한다 (동시 시작이 서로의 세션을 덮어쓰지 않게)
    start_lock: Mutex<()>,
    /// `cancel` 이 올린다 - 시작이 끝날 때 값이 달라졌으면 그 사이 취소된 것이다
    generation: AtomicU64,
}

impl CliLogin {
    pub fn new(host: Arc<CliHost>, opener: Arc<dyn UrlOpener>) -> Self {
        Self::with_timings(host, opener, LoginTimings::default())
    }

    pub fn with_timings(
        host: Arc<CliHost>,
        opener: Arc<dyn UrlOpener>,
        timings: LoginTimings,
    ) -> Self {
        Self {
            host,
            opener,
            timings,
            session: Mutex::new(None),
            start_lock: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    fn session(&self) -> MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 로그인 시작(재시작). URL 을 잡으면 기본 브라우저로 열고 `Waiting{url}` 을 돌려준다.
    pub fn start(&self) -> LoginState {
        let _single_flight = self.start_lock.lock().unwrap_or_else(|e| e.into_inner());
        let generation = self.generation.load(Ordering::SeqCst);
        if let Some(prev) = self.session().take() {
            prev.stop();
        }
        let Some(exe) = self.host.resolve() else {
            return LoginState::failed("cli_not_found");
        };
        let Ok(dir) = WorkDir::create(&self.host.work_root) else {
            return LoginState::failed("unknown");
        };
        let cmd = CliCommand {
            program: exe,
            args: ["auth", "login", "--claudeai"].map(Into::into).to_vec(),
            cwd: Some(dir.path().to_path_buf()),
            stdin: None,
        };
        let Ok(proc) = self.host.exec().spawn(&cmd) else {
            return LoginState::failed("cli_not_found");
        };

        let shared = Arc::new(Shared::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, cancel, host) = (
                Arc::clone(&shared),
                Arc::clone(&cancel),
                Arc::clone(&self.host),
            );
            let timings = self.timings;
            std::thread::spawn(move || monitor(proc, dir, &shared, &cancel, &host, timings))
        };
        let session = Session {
            shared: Arc::clone(&shared),
            cancel,
            thread: Some(thread),
        };

        // URL 이 나오거나, 자식이 URL 없이 끝나거나, 시간이 다 될 때까지 기다린다
        let deadline = Instant::now() + self.timings.url_timeout;
        let url = {
            let mut g = shared.lock();
            loop {
                if self.generation.load(Ordering::SeqCst) != generation {
                    break None; // 기다리는 사이 취소됐다
                }
                if let Some(u) = &g.url {
                    break Some(u.clone());
                }
                if g.finished.is_some() {
                    break None;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break None;
                }
                // 취소를 알아챌 수 있도록 짧게 나눠 기다린다
                g = shared
                    .changed
                    .wait_timeout(g, left.min(POLL))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        };
        let cancelled = self.generation.load(Ordering::SeqCst) != generation;
        let Some(url) = url.filter(|_| !cancelled) else {
            session.stop();
            return if cancelled {
                LoginState::None
            } else {
                LoginState::failed("login_no_url")
            };
        };
        // 세대 확인과 저장을 같은 잠금 안에서 한다 - `cancel` 은 세대를 올린 뒤 이 잠금으로 세션을 가져가므로, 놓치는 틈이 없다
        {
            let mut slot = self.session();
            if self.generation.load(Ordering::SeqCst) != generation {
                drop(slot);
                session.stop();
                return LoginState::None;
            }
            *slot = Some(session);
        }
        // 열기에 실패해도 URL 은 화면에 표시되므로 사용자가 직접 열 수 있다
        let _ = self.opener.open(&url);
        LoginState::Waiting { url: Some(url) }
    }

    /// 진행 상태. 끝난 결과(`Done`/`Failed`)는 **한 번만** 전달하고 세션을 닫는다.
    pub fn status(&self) -> LoginState {
        let shared = self.session().as_ref().map(|s| Arc::clone(&s.shared));
        let Some(shared) = shared else {
            return LoginState::None;
        };
        let finished = shared.lock().finished.take();
        if let Some(done) = finished {
            if let Some(s) = self.session().take() {
                s.stop();
            }
            return done;
        }
        let url = shared.lock().url.clone();
        LoginState::Waiting { url }
    }

    /// 진행 중인 시작과 저장된 세션을 모두 취소한다 (로그아웃·인증 방식 전환도 이 경로).
    pub fn cancel(&self) -> LoginState {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(s) = self.session().take() {
            s.stop();
        }
        LoginState::None
    }

    /// 캡처해 둔 URL 만 다시 연다 (진행 중인 세션이 있을 때). 열었으면 true.
    pub fn open_current_url(&self) -> bool {
        let url = {
            let slot = self.session();
            let Some(s) = slot.as_ref() else {
                return false;
            };
            let g = s.shared.lock();
            if g.finished.is_some() {
                return false;
            }
            g.url.clone()
        };
        url.is_some_and(|u| self.opener.open(&u).is_ok())
    }
}

/// 감시 스레드: 출력에서 URL 을 잡고, 끝나면 `auth status` 로 판정한다. 취소되면 PID 로 종료한다.
fn monitor(
    mut proc: super::exec::Tracked,
    dir: WorkDir,
    shared: &Shared,
    cancel: &AtomicBool,
    host: &CliHost,
    timings: LoginTimings,
) {
    let _dir = dir; // 끝날 때 폴더 정리
    let url_join = timings.url_join;
    let began = Instant::now();
    let mut pending: Option<(String, Instant)> = None;
    loop {
        if cancel.load(Ordering::SeqCst) || host.exec().is_cancelled() {
            proc.kill();
            return;
        }
        if began.elapsed() >= timings.session_timeout {
            // 승인 없이 방치됐다 - 프로세스를 PID 로 종료하고 실패로 알린다 (URL 을 못 받았으면 login_no_url)
            proc.kill();
            let mut g = shared.lock();
            let code = if g.url.is_some() {
                "unknown"
            } else {
                "login_no_url"
            };
            g.finished = Some(LoginState::failed(code));
            shared.changed.notify_all();
            return;
        }
        match proc.next_event(POLL) {
            ProcEvent::Stdout(s) | ProcEvent::Stderr(s) => {
                let candidate = {
                    let mut g = shared.lock();
                    g.out.push_str(&s);
                    if g.out.len() > OUT_KEEP {
                        let cut = g.out.len() - OUT_KEEP;
                        let cut = (cut..g.out.len())
                            .find(|i| g.out.is_char_boundary(*i))
                            .unwrap_or(g.out.len());
                        g.out.drain(..cut);
                    }
                    last_authorize_url(&normalize_output(&g.out))
                };
                if let Some(u) = candidate {
                    let current = shared.lock().url.clone();
                    if current.as_deref() != Some(u.as_str()) {
                        // 줄바꿈된 나머지가 이어질 수 있어 잠시 가라앉힌 뒤 확정한다
                        pending = Some((u, Instant::now()));
                    }
                }
            }
            ProcEvent::Idle => {
                if let Some((u, at)) = &pending
                    && at.elapsed() >= url_join
                {
                    commit_url(shared, u.clone());
                    pending = None;
                }
            }
            ProcEvent::Exit(code) => {
                if let Some((u, _)) = pending.take() {
                    commit_url(shared, u);
                }
                let outcome = outcome_of(host, code);
                let mut g = shared.lock();
                g.finished = Some(outcome);
                shared.changed.notify_all();
                return;
            }
        }
    }
}

fn commit_url(shared: &Shared, url: String) {
    let mut g = shared.lock();
    g.url = Some(url);
    shared.changed.notify_all();
}

/// 종료 판정: 상태 명령 결과 우선, 상태 명령이 실패했을 때만 종료 코드를 근거로 한다.
fn outcome_of(host: &CliHost, code: Option<i32>) -> LoginState {
    let account: Option<CliAccount> = host.resolve().and_then(|exe| host.read_account(&exe));
    match account {
        Some(a) if a.logged_in => LoginState::Done { verified: true },
        Some(_) => LoginState::failed("cli_not_logged_in"),
        None if code == Some(0) => LoginState::Done { verified: false },
        None => LoginState::failed("unknown"),
    }
}
