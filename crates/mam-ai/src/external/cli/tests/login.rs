//! 브라우저 로그인 연동 시험 — 가짜 실행기 + 가짜 브라우저 오프너.
//! 분류: Happy / Boundary(URL 줄바꿈) / Validation(https 만) / External / Concurrency(재시작·취소) / Business rule(코드 중계 없음)

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::external::cli::fake::FakeRunner;
use crate::external::cli::{CliLogin, LoginState, LoginTimings, UrlOpener};

pub(super) const URL: &str =
    "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&response_type=code&state=xyz";

#[derive(Default)]
pub(super) struct Opener {
    pub(super) opened: Mutex<Vec<String>>,
    pub(super) fail: AtomicBool,
}

impl UrlOpener for Opener {
    fn open(&self, url: &str) -> io::Result<()> {
        self.opened.lock().unwrap().push(url.to_string());
        if self.fail.load(Ordering::SeqCst) {
            Err(io::Error::other("no browser"))
        } else {
            Ok(())
        }
    }
}

pub(super) struct Fx {
    pub(super) _dir: tempfile::TempDir,
    pub(super) runner: Arc<FakeRunner>,
    pub(super) host: Arc<CliHost>,
    pub(super) opener: Arc<Opener>,
    pub(super) login: CliLogin,
}

pub(super) fn fx(scripts: Scripts, env: FakeEnv) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(scripts);
    let host = host(Arc::clone(&runner), env, dir.path());
    let opener = Arc::new(Opener::default());
    let login = CliLogin::with_timings(
        Arc::clone(&host),
        Arc::clone(&opener) as _,
        LoginTimings {
            url_timeout: Duration::from_millis(300),
            url_join: Duration::from_millis(30),
            ..LoginTimings::default()
        },
    );
    Fx {
        _dir: dir,
        runner,
        host,
        opener,
        login,
    }
}

/// 끝난 결과가 나올 때까지 (최대 5초) 상태를 본다.
pub(super) fn wait_final(login: &CliLogin) -> LoginState {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = login.status();
        if !matches!(s, LoginState::Waiting { .. }) || Instant::now() >= deadline {
            return s;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn login_script(url_out: &str, gate: &Arc<AtomicBool>, exit: i32) -> Vec<Step> {
    vec![
        Step::Out(url_out.into()),
        Step::Out("Paste code here if prompted > ".into()),
        Step::Gate(Arc::clone(gate)),
        Step::Exit(Some(exit)),
    ]
}

#[test]
fn t_aicli_login_01_auto_completes_without_a_code_and_delivers_done_once() {
    // Arrange: 사용자가 브라우저에서 승인하면 CLI 가 스스로 종료한다
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("Opening browser...\n{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());

    // Act 1: 시작 -> URL 을 잡아 기본 브라우저로 연다
    let started = f.login.start();
    assert_eq!(
        started,
        LoginState::Waiting {
            url: Some(URL.into())
        }
    );
    assert_eq!(*f.opener.opened.lock().unwrap(), vec![URL.to_string()]);
    assert_eq!(
        f.login.status(),
        LoginState::Waiting {
            url: Some(URL.into())
        }
    );

    // Act 2: 승인(CLI 종료)
    gate.store(true, Ordering::SeqCst);
    let done = wait_final(&f.login);

    // Assert: 결과는 한 번만 전달된다
    assert_eq!(done, LoginState::Done { verified: true });
    assert_eq!(f.login.status(), LoginState::None);
    // 코드 중계 경로 없음: stdin 은 닫혀 있고 인자는 고정이다
    let cmd = f
        .runner
        .spawned
        .lock()
        .unwrap()
        .iter()
        .find(|c| is_cmd(c, "auth") && args_of(c)[1] == "login")
        .cloned()
        .unwrap();
    assert_eq!(args_of(&cmd), vec!["auth", "login", "--claudeai"]);
    assert!(cmd.stdin.is_none());
}

#[test]
fn t_aicli_login_02_joins_a_url_wrapped_across_lines() {
    // Arrange: 첫 조각에서 URL 이 끊기고 나머지가 다음 출력으로 온다
    let (a, b) = URL.split_once("&response_type").unwrap();
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: vec![
            Step::Out(format!("visit:\n{a}\n")),
            Step::Out(format!("  &response_type{b}\n")),
            Step::Gate(gate),
        ],
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());

    // Act
    let st = f.login.start();

    // Assert: 끊긴 앞부분이 아니라 이어 붙인 전체를 열었다
    assert_eq!(
        st,
        LoginState::Waiting {
            url: Some(URL.into())
        }
    );
    assert_eq!(*f.opener.opened.lock().unwrap(), vec![URL.to_string()]);
    f.login.cancel();
}

#[test]
fn t_aicli_login_03_non_https_urls_are_never_opened() {
    for out in [
        "http://claude.com/cai/oauth/authorize?code=true\n",
        "file:///C:/Windows/System32/cmd.exe\n",
        "https://docs.example.com/help\n",
    ] {
        let s = Scripts {
            login: vec![Step::Out(out.into()), Step::Hang],
            ..Default::default()
        };
        let f = fx(s, FakeEnv::with_claude());

        let st = f.login.start();

        assert_eq!(
            st,
            LoginState::Failed {
                error_code: "login_no_url".into()
            },
            "{out:?}"
        );
        assert!(f.opener.opened.lock().unwrap().is_empty());
        assert_eq!(
            f.runner.killed_pids().len(),
            1,
            "URL 없이 시간 초과 -> PID 종료"
        );
    }
}

#[test]
fn t_aicli_login_04_no_url_before_timeout_fails_and_kills_the_cli() {
    let s = Scripts {
        login: vec![Step::Out("Starting login...\n".into()), Step::Hang],
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());

    let st = f.login.start();

    assert_eq!(
        st,
        LoginState::Failed {
            error_code: "login_no_url".into()
        }
    );
    assert_eq!(f.runner.killed_pids().len(), 1);
    assert_eq!(f.login.status(), LoginState::None, "세션이 남지 않는다");
    assert_eq!(f.host.exec().active_count(), 0);
}

#[test]
fn t_aicli_login_05_cli_exiting_without_a_url_is_login_no_url() {
    let s = Scripts {
        login: vec![Step::Out("error: something\n".into()), Step::Exit(Some(1))],
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());

    assert_eq!(
        f.login.start(),
        LoginState::Failed {
            error_code: "login_no_url".into()
        }
    );
}

#[test]
fn t_aicli_login_06_restart_kills_the_previous_session() {
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    assert!(matches!(f.login.start(), LoginState::Waiting { .. }));
    assert!(f.runner.killed_pids().is_empty());

    let second = f.login.start();

    assert!(matches!(second, LoginState::Waiting { .. }));
    assert_eq!(
        f.runner.killed_pids().len(),
        1,
        "이전 세션 프로세스를 PID 로 종료"
    );
    assert_eq!(f.runner.count_with_arg("--claudeai"), 2);
    assert_eq!(f.host.exec().active_count(), 1, "단일 세션만 남는다");
    f.login.cancel();
    assert_eq!(f.host.exec().active_count(), 0);
}

#[test]
fn t_aicli_login_07_cancel_kills_and_resets_state() {
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    f.login.start();

    assert_eq!(f.login.cancel(), LoginState::None);

    assert_eq!(f.runner.killed_pids().len(), 1);
    assert_eq!(f.login.status(), LoginState::None);
    assert!(
        !f.login.open_current_url(),
        "취소 뒤에는 URL 을 다시 열지 않는다"
    );
    assert_eq!(f.login.cancel(), LoginState::None, "세션이 없어도 안전");
}

#[test]
fn t_aicli_login_08_status_command_decides_the_outcome() {
    // 상태 명령이 로그인 아님 -> 실패, 실패(알 수 없음) + 종료 코드 0 -> 미검증 완료, 실패 + 비정상 종료 -> unknown
    let cases: [(Vec<Step>, i32, LoginState); 3] = [
        (
            vec![
                Step::Out(r#"{"loggedIn":false}"#.into()),
                Step::Exit(Some(0)),
            ],
            0,
            LoginState::Failed {
                error_code: "cli_not_logged_in".into(),
            },
        ),
        (
            vec![Step::Out("help text".into()), Step::Exit(Some(1))],
            0,
            LoginState::Done { verified: false },
        ),
        (
            vec![Step::Exit(Some(1))],
            1,
            LoginState::Failed {
                error_code: "unknown".into(),
            },
        ),
    ];
    for (status, exit, expect) in cases {
        let s = Scripts {
            status,
            login: vec![Step::Out(format!("{URL}\n")), Step::Exit(Some(exit))],
            ..Default::default()
        };
        let f = fx(s, FakeEnv::with_claude());
        assert!(matches!(f.login.start(), LoginState::Waiting { .. }));

        assert_eq!(wait_final(&f.login), expect);
    }
}

#[test]
fn t_aicli_login_09_cli_not_found_or_unspawnable_fails_with_cli_not_found() {
    let f = fx(Scripts::default(), FakeEnv::without_claude());
    assert_eq!(
        f.login.start(),
        LoginState::Failed {
            error_code: "cli_not_found".into()
        }
    );
    assert_eq!(f.runner.spawn_count(), 0);

    let runner = FakeRunner::new(|_| None);
    let dir = tempfile::tempdir().unwrap();
    let h = host(runner, FakeEnv::with_claude(), dir.path());
    let login = CliLogin::new(h, Arc::new(Opener::default()));
    assert_eq!(
        login.start(),
        LoginState::Failed {
            error_code: "cli_not_found".into()
        }
    );
}

#[test]
fn t_aicli_login_10_browser_open_failure_still_shows_the_url_and_reopen_works() {
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    f.opener.fail.store(true, Ordering::SeqCst);

    let st = f.login.start();

    assert_eq!(
        st,
        LoginState::Waiting {
            url: Some(URL.into())
        }
    );
    f.opener.fail.store(false, Ordering::SeqCst);
    assert!(f.login.open_current_url());
    assert_eq!(
        f.opener.opened.lock().unwrap().len(),
        2,
        "캡처한 URL 만 다시 연다"
    );
    assert!(f.opener.opened.lock().unwrap().iter().all(|u| u == URL));
    f.login.cancel();
}

#[test]
fn t_aicli_login_11_login_uses_an_empty_work_dir_that_is_removed_afterwards() {
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    f.login.start();
    let root = f.host.work_root.clone();
    assert_eq!(files_in(&root), 1);

    f.login.cancel();

    assert_eq!(files_in(&root), 0);
}

#[test]
fn t_aicli_login_12_kill_all_stops_a_waiting_session() {
    let gate = Arc::new(AtomicBool::new(false));
    let s = Scripts {
        login: login_script(&format!("{URL}\n"), &gate, 0),
        ..Default::default()
    };
    let f = fx(s, FakeEnv::with_claude());
    f.login.start();

    f.host.kill_all();
    f.login.cancel(); // 감시 스레드 회수

    assert!(!f.runner.killed_pids().is_empty());
    assert_eq!(f.host.exec().active_count(), 0);
}

#[test]
fn t_aicli_login_13_state_serializes_the_ipc_contract_shapes() {
    let j = |s: &LoginState| serde_json::to_value(s).unwrap();
    assert_eq!(j(&LoginState::None), serde_json::json!({"state": "none"}));
    assert_eq!(
        j(&LoginState::Waiting {
            url: Some(URL.into())
        }),
        serde_json::json!({"state": "waiting", "url": URL})
    );
    assert_eq!(
        j(&LoginState::Waiting { url: None }),
        serde_json::json!({"state": "waiting", "url": null})
    );
    assert_eq!(
        j(&LoginState::Done { verified: false }),
        serde_json::json!({"state": "done", "verified": false})
    );
    assert_eq!(
        j(&LoginState::Failed {
            error_code: "login_no_url".into()
        }),
        serde_json::json!({"state": "failed", "errorCode": "login_no_url"})
    );
}
