//! `CliClient` 시험 — 가짜 실행기로 happy / 미설치 / 버전 / 로그인 / 타임아웃(PID 종료) / 출력 손상 / 한도 / 오프라인 조기 중단 /
//! 작업 폴더 정리 / 전송 범위 / 직렬화 / 사용량 기록.
//! 분류: Happy / Validation / External / Business rule / Concurrency / Time

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::external::cli::CliClient;
use crate::external::cli::fake::FakeRunner;
use crate::external::client::{AiClient, check_provider};
use crate::external::consent::ConsentGate;
use crate::external::error::ExternalError;
use crate::external::types::{AuthMode, UsageRecord, UsageSink};

#[derive(Default)]
struct Sink(Mutex<Vec<UsageRecord>>);
impl UsageSink for Sink {
    fn record(&self, u: &UsageRecord) {
        self.0.lock().unwrap().push(u.clone());
    }
}

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

struct Fx {
    dir: tempfile::TempDir,
    runner: Arc<crate::external::cli::fake::FakeRunner>,
    host: Arc<CliHost>,
    sink: Arc<Sink>,
}

impl Fx {
    fn new(scripts: Scripts, env: FakeEnv) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runner = dispatch(scripts);
        let host = host(Arc::clone(&runner), env, dir.path());
        Self {
            dir,
            runner,
            host,
            sink: Arc::default(),
        }
    }

    fn client(&self) -> CliClient {
        CliClient::new(Arc::clone(&self.host), Some(Arc::clone(&self.sink) as _))
    }

    fn work_root(&self) -> std::path::PathBuf {
        self.dir.path().join("tmp").join("ai-cli")
    }

    fn request_cmd(&self) -> CliCommand {
        self.runner
            .spawned
            .lock()
            .unwrap()
            .iter()
            .find(|c| is_cmd(c, "-p"))
            .cloned()
            .expect("request spawned")
    }

    fn last_usage(&self) -> UsageRecord {
        self.sink.0.lock().unwrap().last().cloned().expect("usage")
    }
}

#[test]
fn t_aicli_01_happy_returns_structured_output_and_subscription_usage() {
    // Arrange
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());

    // Act
    let out = fx.client().analyze(&gate(), &request()).unwrap();

    // Assert
    assert_eq!(out.description, "붉은 노을");
    assert_eq!(out.keywords, vec!["노을", "하늘"]);
    assert_eq!(out.summary, "노을 사진");
    assert_eq!(out.language, "ko");
    assert_eq!(out.model, "claude-haiku-5-5", "CLI 가 실제로 쓴 모델");
    let u = fx.last_usage();
    assert!(u.ok && u.error_code.is_none());
    assert_eq!(u.input_tokens, 2 + 7187);
    assert_eq!(u.output_tokens, 1317);
    assert_eq!(u.auth_mode, AuthMode::Subscription);
    assert_eq!(u.asset_id, Some(7));
    assert_eq!(out.usage, u);
}

#[test]
fn t_aicli_02_request_carries_only_prepared_image_and_sanitized_meta() {
    // Arrange
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());

    // Act
    fx.client().analyze(&gate(), &request()).unwrap();

    // Assert: 인자·cwd·stdin 어디에도 원본 경로가 없다
    let cmd = fx.request_cmd();
    let args = args_of(&cmd).join("\u{1}");
    let stdin = String::from_utf8(cmd.stdin.clone().unwrap()).unwrap();
    for hay in [
        &args,
        &stdin,
        &cmd.cwd.clone().unwrap().to_string_lossy().into_owned(),
    ] {
        assert!(
            !hay.contains("secret-trip")
                && !hay.contains(r"D:\\Photos")
                && !hay.contains(r"D:\Photos")
        );
    }
    assert!(
        stdin.contains("IMG_0001.jpg"),
        "정제된 파일 이름은 프롬프트에 들어간다"
    );
    let v: serde_json::Value = serde_json::from_str(stdin.trim()).unwrap();
    let content = v["message"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "텍스트 + 준비된 이미지 1장");
    assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
    // 도구 비활성 플래그 고정 + 기본 모델
    let a = args_of(&cmd);
    let t = a.iter().position(|s| s == "--tools").unwrap();
    assert_eq!(a[t + 1], "");
    let m = a.iter().position(|s| s == "--model").unwrap();
    assert_eq!(a[m + 1], "haiku");
    assert!(cmd.program.ends_with("claude.exe"));
}

#[test]
fn t_aicli_03_work_dir_is_a_fresh_empty_dir_under_mam_data_and_removed_afterwards() {
    // Arrange: 실행 시점의 cwd 상태를 가짜 실행기가 직접 본다
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tmp").join("ai-cli");
    let seen_empty = Arc::new(AtomicBool::new(false));
    let (flag, root2) = (Arc::clone(&seen_empty), root.clone());
    let runner = FakeRunner::new(move |c| match args_of(c).first().map(String::as_str) {
        Some("--version") => Some(vec![
            Step::Out(
                "2.1.318
"
                .into(),
            ),
            Step::Exit(Some(0)),
        ]),
        Some("-p") => {
            let cwd = c.cwd.clone().unwrap();
            flag.store(
                cwd.starts_with(&root2) && cwd != root2 && files_in(&cwd) == 0,
                Ordering::SeqCst,
            );
            Some(success_request())
        }
        _ => None,
    });
    let h = host(runner, FakeEnv::with_claude(), dir.path());

    // Act
    CliClient::new(h, None)
        .analyze(&gate(), &request())
        .unwrap();

    // Assert
    assert!(seen_empty.load(Ordering::SeqCst));
    assert_eq!(files_in(&root), 0, "성공 뒤 작업 폴더가 남지 않는다");
}

#[test]
fn t_aicli_04_cli_not_found_never_spawns() {
    let fx = Fx::new(Scripts::default(), FakeEnv::without_claude());

    let e = fx.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(e, ExternalError::CliNotFound);
    assert_eq!(fx.runner.spawn_count(), 0);
    assert_eq!(fx.last_usage().error_code.as_deref(), Some("cli_not_found"));
}

#[test]
fn t_aicli_05_unsupported_version_is_refused_before_the_request() {
    for ver in ["2.1.279 (Claude Code)\n", "1.0.0\n", "garbage\n"] {
        let s = Scripts {
            version: Some(vec![Step::Out(ver.into()), Step::Exit(Some(0))]),
            ..Default::default()
        };
        let fx = Fx::new(s, FakeEnv::with_claude());

        let e = fx.client().analyze(&gate(), &request()).unwrap_err();

        assert_eq!(e, ExternalError::CliUnsupportedVersion, "{ver:?}");
        assert_eq!(fx.runner.count_with_arg("-p"), 0);
    }
}

#[test]
fn t_aicli_06_version_probe_failures_map_to_not_found_or_unsupported() {
    // 실행 불가 -> 찾을 수 없음, 비정상 종료 -> 지원 안 함
    let s = Scripts {
        version: None,
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());
    assert_eq!(
        fx.client().analyze(&gate(), &request()).unwrap_err(),
        ExternalError::CliNotFound
    );
    let s = Scripts {
        version: Some(vec![Step::Exit(Some(1))]),
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());
    assert_eq!(
        fx.client().analyze(&gate(), &request()).unwrap_err(),
        ExternalError::CliUnsupportedVersion
    );
}

#[test]
fn t_aicli_07_version_is_probed_once_per_executable() {
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());
    let c = fx.client();
    c.analyze(&gate(), &request()).unwrap();
    c.analyze(&gate(), &request()).unwrap();
    assert_eq!(fx.runner.count_with_arg("--version"), 1);
    assert_eq!(fx.runner.count_with_arg("-p"), 2);
}

#[test]
fn t_aicli_08_not_logged_in_result_maps_to_cli_not_logged_in() {
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(&error_result("null", "Not logged in · Please run /login")),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());

    let e = fx.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(e, ExternalError::CliNotLoggedIn);
    assert_eq!(e.check_code(), "cli_not_logged_in");
    assert!(!fx.last_usage().ok);
}

#[test]
fn t_aicli_09_timeout_kills_the_process_by_pid_and_cleans_up() {
    // Arrange: 결과 없이 멈춘 CLI
    let s = Scripts {
        request: vec![line(INIT_SUB), Step::Hang],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());
    let started = Instant::now();

    // Act
    let e = fx
        .client()
        .with_cap(Duration::from_millis(200))
        .analyze(&gate(), &request())
        .unwrap_err();

    // Assert
    assert_eq!(e, ExternalError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        fx.runner.killed_pids().len(),
        1,
        "요청 프로세스를 PID 로 종료"
    );
    assert_eq!(files_in(&fx.work_root()), 0);
    assert_eq!(
        fx.host.exec().active_count(),
        0,
        "PID 추적 목록이 비어 있다"
    );
}

#[test]
fn t_aicli_10_malformed_outputs_are_bad_response_without_leaking_content() {
    for script in [
        // 결과가 JSON 이 아니다
        vec![
            line(INIT_SUB),
            line(
                r#"{"type":"result","subtype":"success","is_error":false,"result":"sorry C:\\Users\\me\\x"}"#,
            ),
            Step::Exit(Some(0)),
        ],
        // 결과 줄 없이 종료
        vec![line(INIT_SUB), Step::Exit(Some(1))],
        // 아무것도 없이 종료
        vec![Step::Out("garbage\n".into()), Step::Exit(Some(0))],
    ] {
        let s = Scripts {
            request: script,
            ..Default::default()
        };
        let fx = Fx::new(s, FakeEnv::with_claude());

        let e = fx.client().analyze(&gate(), &request()).unwrap_err();

        assert!(matches!(e, ExternalError::BadResponse(_)), "{e:?}");
        assert!(!format!("{e:?}{}", e.user_message()).contains("Users"));
    }
}

#[test]
fn t_aicli_11_rejected_rate_limit_maps_to_subscription_limit_with_retry_after_until_reset() {
    // 주간 한도가 3일 뒤에 풀리면 대기도 3일이다 - 작업은 시도 횟수 없이 그때까지 미뤄진다 (clock = 1_000_000_000 초)
    let resets = 1_000_000_000i64 + 3 * 86_400;
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(&format!(
                r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"rejected","resetsAt":{resets}}}}}"#
            )),
            line(&error_result("429", "limit")),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());

    let e = fx.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(
        e,
        ExternalError::SubscriptionLimit {
            retry_after_secs: Some(3 * 86_400)
        }
    );
    assert_eq!(e.error_code(), "subscription_limit");
}

#[test]
fn t_aicli_12_two_null_status_retries_abort_early_as_offline() {
    // Arrange: 재시도만 반복하고 끝나지 않는 CLI (실측: 170초)
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(RETRY_NULL),
            line(RETRY_NULL),
            Step::Hang,
        ],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());
    let started = Instant::now();

    // Act: 제한 시간(기본 120초)을 기다리지 않는다
    let e = fx.client().analyze(&gate(), &request()).unwrap_err();

    // Assert
    assert_eq!(e, ExternalError::Offline);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fx.runner.killed_pids().len(), 1);
    assert_eq!(files_in(&fx.work_root()), 0);
}

#[test]
fn t_aicli_13_api_key_source_other_than_none_is_recorded_as_api_key_billing() {
    let s = Scripts {
        request: vec![line(INIT_KEY), line(OK_RESULT), Step::Exit(Some(0))],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude().with_var("ANTHROPIC_API_KEY"));

    fx.client().analyze(&gate(), &request()).unwrap();

    assert_eq!(fx.last_usage().auth_mode, AuthMode::ApiKey);
}

#[test]
fn t_aicli_14_environment_is_not_modified_by_the_client() {
    // 환경변수를 지우거나 덮어쓰는 인자·경로가 없다 — 실행 명령은 argv/cwd/stdin 뿐이다 (CliCommand 에 env 필드 없음)
    let fx = Fx::new(
        Scripts::default(),
        FakeEnv::with_claude().with_var("ANTHROPIC_API_KEY"),
    );
    fx.client().analyze(&gate(), &request()).unwrap();
    let cmd = fx.request_cmd();
    assert!(!args_of(&cmd).iter().any(|a| a.contains("ANTHROPIC")));
}

#[test]
fn t_aicli_15_check_sends_text_only_without_schema_and_reports_ok() {
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(
                r#"{"type":"result","subtype":"success","is_error":false,"result":"ok","usage":{"input_tokens":5,"output_tokens":1}}"#,
            ),
            Step::Exit(Some(0)),
        ],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());

    let r = check_provider(&gate(), &fx.client(), None);

    assert!(r.ok && r.error_code.is_none());
    let cmd = fx.request_cmd();
    assert!(!args_of(&cmd).contains(&"--json-schema".to_string()));
    let v: serde_json::Value = serde_json::from_slice(&cmd.stdin.unwrap()).unwrap();
    assert_eq!(v["message"]["content"].as_array().unwrap().len(), 1);
    let u = fx.last_usage();
    assert_eq!(u.kind, "check");
    assert_eq!(u.auth_mode, AuthMode::Subscription);
}

#[test]
fn t_aicli_16_check_failures_use_contract_error_codes() {
    for (scripts, env, code) in [
        (
            Scripts::default(),
            FakeEnv::without_claude(),
            "cli_not_found",
        ),
        (
            Scripts {
                version: Some(vec![Step::Out("2.0.0\n".into()), Step::Exit(Some(0))]),
                ..Default::default()
            },
            FakeEnv::with_claude(),
            "cli_unsupported_version",
        ),
        (
            Scripts {
                request: vec![
                    line(INIT_SUB),
                    line(RETRY_NULL),
                    line(RETRY_NULL),
                    Step::Hang,
                ],
                ..Default::default()
            },
            FakeEnv::with_claude(),
            "offline",
        ),
    ] {
        let fx = Fx::new(scripts, env);
        let r = check_provider(&gate(), &fx.client(), None);
        assert!(!r.ok);
        assert_eq!(r.error_code.as_deref(), Some(code));
    }
}

#[test]
fn t_aicli_17_invalid_model_and_request_are_rejected_before_any_spawn() {
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());
    let mut bad_model = request();
    bad_model.model = Some("evil; rm -rf".into());
    assert!(matches!(
        fx.client().analyze(&gate(), &bad_model).unwrap_err(),
        ExternalError::InvalidInput(_)
    ));
    let mut no_image = request();
    no_image.images.clear();
    assert!(matches!(
        fx.client().analyze(&gate(), &no_image).unwrap_err(),
        ExternalError::InvalidInput(_)
    ));
    assert_eq!(fx.runner.spawn_count(), 0);
}

#[test]
fn t_aicli_18_user_model_is_passed_through_to_the_model_flag() {
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());
    let mut req = request();
    req.model = Some("claude-sonnet-5".into());

    fx.client().analyze(&gate(), &req).unwrap();

    let a = args_of(&fx.request_cmd());
    let m = a.iter().position(|s| s == "--model").unwrap();
    assert_eq!(a[m + 1], "claude-sonnet-5");
}

#[test]
fn t_aicli_19_requests_are_serialized_one_at_a_time() {
    // Arrange: 첫 요청이 상한(300ms)까지 멈춰 있다. 둘째는 그 뒤에야 실행돼야 한다.
    let starts: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let st = Arc::clone(&starts);
    let first = Arc::new(AtomicBool::new(true));
    let runner = crate::external::cli::fake::FakeRunner::new(move |c| {
        let a = args_of(c);
        match a.first().map(String::as_str) {
            Some("--version") => Some(vec![Step::Out("2.1.318\n".into()), Step::Exit(Some(0))]),
            Some("-p") => {
                st.lock().unwrap().push(Instant::now());
                if first.swap(false, Ordering::SeqCst) {
                    Some(vec![Step::Hang])
                } else {
                    Some(success_request())
                }
            }
            _ => None,
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let h = host(runner, FakeEnv::with_claude(), dir.path());
    let c1 = Arc::new(CliClient::new(Arc::clone(&h), None).with_cap(Duration::from_millis(300)));
    let c2 = Arc::new(CliClient::new(Arc::clone(&h), None));

    // Act
    let t1 = {
        let c = Arc::clone(&c1);
        std::thread::spawn(move || c.analyze(&gate(), &request()))
    };
    std::thread::sleep(Duration::from_millis(80));
    let r2 = c2.analyze(&gate(), &request());
    let r1 = t1.join().unwrap();

    // Assert
    assert_eq!(r1.unwrap_err(), ExternalError::Timeout);
    assert!(r2.is_ok());
    let s = starts.lock().unwrap();
    assert_eq!(s.len(), 2);
    assert!(
        s[1].duration_since(s[0]) >= Duration::from_millis(250),
        "둘째 요청은 첫째가 끝난 뒤에 시작"
    );
}

#[test]
fn t_aicli_20_kill_all_stops_a_running_request_and_blocks_new_ones() {
    // Arrange
    let s = Scripts {
        request: vec![line(INIT_SUB), Step::Hang],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());
    let client = Arc::new(fx.client());
    let t = {
        let c = Arc::clone(&client);
        std::thread::spawn(move || c.analyze(&gate(), &request()))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.runner.count_with_arg("-p") == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }

    // Act
    fx.host.kill_all();
    let r = t.join().unwrap();

    // Assert
    assert_eq!(r.unwrap_err(), ExternalError::Cancelled);
    assert!(!fx.runner.killed_pids().is_empty());
    let again = client.analyze(&gate(), &request()).unwrap_err();
    assert_eq!(again, ExternalError::Cancelled);
    assert_eq!(
        fx.runner.count_with_arg("-p"),
        1,
        "종료 뒤 새 요청은 실행되지 않는다"
    );
}

#[test]
fn t_aicli_21_consent_gate_cannot_be_built_without_consent_so_nothing_spawns() {
    let fx = Fx::new(Scripts::default(), FakeEnv::with_claude());
    assert_eq!(
        ConsentGate::check(false).unwrap_err(),
        ExternalError::ConsentRequired
    );
    assert_eq!(fx.runner.spawn_count(), 0);
}

#[test]
fn t_aicli_22_usage_is_recorded_for_failures_too() {
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(&error_result("500", "x")),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let fx = Fx::new(s, FakeEnv::with_claude());

    let e = fx.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(e, ExternalError::Server { status: 500 });
    let u = fx.last_usage();
    assert!(!u.ok);
    assert_eq!(u.error_code.as_deref(), Some("server_error"));
    assert_eq!(u.auth_mode, AuthMode::Subscription);
    assert!(u.request_bytes > 0);
}
