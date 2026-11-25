// 구독 한도 일시정지·실제 과금 방식 기록 연결 시험 — 실제 `claude` 는 띄우지 않는다(가짜 실행기).
// Categories: Business rule (한도 중 프로세스 0개, 해제 시각 이후 재개, API 키 모드는 무관) / Time (시계 주입) /
// External (환경변수 키로 과금된 실패)
use std::sync::atomic::Ordering;

use mam_ai::external::cli::fake::Step;
use mam_core::ai_jobs::ExternalAnalyzer;

use super::cli_tests::{adapter, request, subscription};
use super::test_support::{Fx, cli_line};

const NOW_MS: i64 = 1_800_000_000_000;

fn limited_request(resets_at: i64) -> Vec<Step> {
    vec![
        cli_line(r#"{"type":"system","subtype":"init","apiKeySource":"none"}"#),
        cli_line(&format!(
            r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"rejected","resetsAt":{resets_at}}}}}"#
        )),
        cli_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"x"}"#,
        ),
        Step::Exit(Some(1)),
    ]
}

fn ready() -> (Fx, std::path::PathBuf) {
    let fx = Fx::with(super::test_support::StubTransport::new(200, "{}"), NOW_MS);
    fx.insert_asset(1, "image", "a.jpg");
    fx.service.give_consent().unwrap();
    subscription(&fx);
    let thumb = fx.write_thumb("t1.jpg");
    (fx, thumb)
}

#[test]
fn t_aicli_lim_svc_01_limit_pauses_the_adapter_and_later_jobs_spawn_nothing_until_reset() {
    // Arrange: 2시간 뒤에 풀리는 한도
    let (fx, thumb) = ready();
    let a = adapter(&fx);
    fx.cli.scripts.lock().unwrap().request = limited_request(NOW_MS / 1000 + 7200);
    assert_eq!(a.paused_until_ms(), None);

    // Act 1: 첫 요청이 한도에 걸린다
    let e = a.analyze(&request(1, vec![thumb.clone()])).unwrap_err();

    // Assert 1: 해제 시각이 실행기에 알려진다
    assert_eq!(e.code, "subscription_limit");
    assert_eq!(e.retry_after_secs, Some(7200));
    assert_eq!(a.paused_until_ms(), Some(NOW_MS + 7_200_000));
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 1);

    // Act 2: 일시정지 중 다시 시도해도 프로세스를 띄우지 않는다
    let e = a.analyze(&request(1, vec![thumb.clone()])).unwrap_err();
    assert_eq!(e.code, "subscription_limit");
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 1);

    // Act 3: 해제 시각이 지나면 (한도가 풀린 응답으로) 재개된다
    fx.cli.scripts.lock().unwrap().request = super::test_support::CliScripts::default().request;
    fx.clock.0.fetch_add(7_200_000, Ordering::SeqCst);
    assert_eq!(a.paused_until_ms(), None);
    let out = a.analyze(&request(1, vec![thumb])).unwrap();
    assert_eq!(out.provider, "claude");
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 2);
}

#[test]
fn t_aicli_lim_svc_02_api_key_mode_ignores_a_cli_pause() {
    // Arrange: CLI 호스트는 멈춰 있지만 공급자는 API 키 모드
    let fx = Fx::with(super::test_support::StubTransport::new(200, "{}"), NOW_MS);
    fx.service.cli.pause_for_secs(3600);
    subscription(&fx); // 기본 공급자를 claude 로 만든 뒤
    fx.service
        .set_provider_auth_mode("claude", "apiKey")
        .unwrap(); // API 키 모드로 되돌린다

    // Act + Assert: 구독 모드가 아니면 일시정지를 알리지 않는다 (API 키 요청은 별개 경로)
    assert_eq!(adapter(&fx).paused_until_ms(), None);
}

#[test]
fn t_aicli_lim_svc_03_failed_cli_request_billed_to_an_env_api_key_carries_that_mode() {
    // Arrange: 구독 모드지만 CLI 가 환경변수 API 키로 실행됐고 401 로 실패한다
    let (fx, thumb) = ready();
    fx.cli.scripts.lock().unwrap().request = vec![
        cli_line(r#"{"type":"system","subtype":"init","apiKeySource":"ANTHROPIC_API_KEY"}"#),
        cli_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"x"}"#,
        ),
        Step::Exit(Some(1)),
    ];

    // Act
    let e = adapter(&fx).analyze(&request(1, vec![thumb])).unwrap_err();

    // Assert: 구독 로그인 문제가 아니라 그 키의 문제, 실제 과금 방식이 실패 정보에 실린다
    assert_eq!(e.code, "auth_failed");
    assert_eq!(e.billed_auth_mode.as_deref(), Some("apiKey"));
}

#[test]
fn t_aicli_lim_svc_04_failure_without_an_env_key_carries_no_billing_override() {
    // 회귀 대조: 구독으로 과금된 실패는 덮어쓰기 없음 (실행기가 설정된 방식을 쓴다)
    let (fx, thumb) = ready();
    fx.cli.scripts.lock().unwrap().request = vec![
        cli_line(r#"{"type":"system","subtype":"init","apiKeySource":"none"}"#),
        cli_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":500,"result":"x"}"#,
        ),
        Step::Exit(Some(1)),
    ];

    let e = adapter(&fx).analyze(&request(1, vec![thumb])).unwrap_err();

    assert_eq!(e.code, "server_error");
    assert_eq!(e.billed_auth_mode, None);
}
