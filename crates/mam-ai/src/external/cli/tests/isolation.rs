//! 사용자 컨텍스트 격리 시험 — 모든 요청 인자에 `--safe-mode` 와 고정 `--system-prompt`, 작업 폴더는 빈 새 폴더(정리됨),
//! 앱 시작 시 남은 폴더 정리, 모델 ID 검증. (실측 근거: `args.rs` 모듈 문서)
//! 분류: Business rule / Validation / External / Boundary

use std::sync::Arc;

use super::*;
use crate::external::cli::CliClient;
use crate::external::cli::args::SYSTEM_PROMPT;
use crate::external::client::AiClient;
use crate::external::consent::ConsentGate;
use crate::external::error::ExternalError;

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

fn request_cmds(runner: &crate::external::cli::fake::FakeRunner) -> Vec<CliCommand> {
    runner
        .spawned
        .lock()
        .unwrap()
        .iter()
        .filter(|c| is_cmd(c, "-p"))
        .cloned()
        .collect()
}

fn assert_isolated(c: &CliCommand) {
    let a = args_of(c);
    assert!(a.contains(&"--safe-mode".to_string()), "{a:?}");
    let sp = a.iter().position(|s| s == "--system-prompt").expect("flag");
    assert_eq!(a[sp + 1], SYSTEM_PROMPT);
    let tools = a.iter().position(|s| s == "--tools").unwrap();
    assert_eq!(a[tools + 1], "", "도구는 계속 꺼져 있다");
}

#[test]
fn t_aicli_iso_01_analyze_and_check_runs_carry_safe_mode_and_the_fixed_system_prompt() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(Scripts::default());
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    let c = CliClient::new(Arc::clone(&h), None);

    // Act
    c.analyze(&gate(), &request()).unwrap();
    c.check(&gate(), None).unwrap();

    // Assert: 분석(스키마 있음)과 연결 확인(스키마 없음) 모두
    let cmds = request_cmds(&runner);
    assert_eq!(cmds.len(), 2);
    for cmd in &cmds {
        assert_isolated(cmd);
    }
    assert!(
        !SYSTEM_PROMPT.contains(['\\', '/']),
        "경로·사용자 정보 없음"
    );
}

#[test]
fn t_aicli_iso_02_each_run_gets_a_fresh_empty_cwd_under_the_work_root_that_is_removed() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(Scripts::default());
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    let root = dir.path().join("tmp").join("ai-cli");
    let c = CliClient::new(h, None);

    // Act
    c.analyze(&gate(), &request()).unwrap();
    c.analyze(&gate(), &request()).unwrap();

    // Assert
    let cwds: Vec<_> = request_cmds(&runner)
        .into_iter()
        .map(|c| c.cwd.expect("cwd"))
        .collect();
    assert_ne!(cwds[0], cwds[1]);
    for cwd in &cwds {
        assert!(cwd.starts_with(&root));
        assert!(!cwd.exists(), "끝난 뒤 지워진다");
    }
    assert_eq!(files_in(&root), 0);
}

#[test]
fn t_aicli_iso_03_startup_purges_run_dirs_left_by_a_crash() {
    // Arrange: 이전 실행이 비정상 종료로 남긴 폴더 + 무관한 폴더
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("tmp").join("ai-cli");
    std::fs::create_dir_all(root.join("run-4242-0-abc")).unwrap();
    std::fs::write(root.join("run-4242-0-abc").join("scratch"), b"x").unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();

    // Act
    let _h = host(
        dispatch(Scripts::default()),
        FakeEnv::with_claude(),
        dir.path(),
    );

    // Assert
    assert!(!root.join("run-4242-0-abc").exists());
    assert!(root.join("other").exists());
}

#[test]
fn t_aicli_iso_04_model_ids_that_look_like_options_are_rejected_without_spawning() {
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(Scripts::default());
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    let c = CliClient::new(h, None);
    let mut req = request();
    req.model = Some("--dangerously-skip-permissions".into());

    let e = c.analyze(&gate(), &req).unwrap_err();
    let e2 = c.check(&gate(), Some("-p")).unwrap_err();

    assert!(matches!(e, ExternalError::InvalidInput(_)), "{e:?}");
    assert!(matches!(e2, ExternalError::InvalidInput(_)), "{e2:?}");
    assert_eq!(runner.spawn_count(), 0);
}

#[test]
fn t_aicli_iso_05_failed_request_reports_the_billing_the_cli_announced() {
    // Arrange: 구독 모드지만 CLI 가 환경변수 API 키를 썼고, 요청은 실패한다
    let s = Scripts {
        request: vec![
            line(INIT_KEY),
            line(&error_result("401", "bad key")),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let h = host(dispatch(s), FakeEnv::with_claude(), dir.path());
    let c = CliClient::new(h, None);
    assert!(!c.last_billed_to_api_key());

    // Act
    let e = c.analyze(&gate(), &request()).unwrap_err();

    // Assert
    assert_eq!(e, ExternalError::AuthFailed);
    assert!(c.last_billed_to_api_key());
}
