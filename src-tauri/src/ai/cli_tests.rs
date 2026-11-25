// 구독 인증(Claude Code CLI 대리 실행) 연결 시험 — 실제 `claude` 는 띄우지 않는다(가짜 실행기).
// Categories: Happy (전환·탐지·확인·분석·로그인) / Validation (claude 외 거부) / Business rule (동의 전 프로세스 0개, 백그라운드 일괄 분석 제외,
// 로그아웃은 진행 중 요청이 있으면 거부, 원본 경로 비전송, 구 설정 호환) / External (CLI 실패 코드)
use std::path::PathBuf;
use std::sync::Arc;

use mam_ai::external::cli::LoginState;
use mam_ai::external::cli::fake::Step;
use mam_core::ai_jobs::{ExternalAnalyzer, ExternalRequest, ImageInput, StepKind, Transfer};
use serde_json::json;

use super::AiFailure;
use super::external::ExternalAdapter;
use super::service::parse_provider;
use super::settings::StoredAi;
use super::test_support::{Fx, StubTransport, cli_line};

const LOGIN_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz";

pub(super) fn adapter(fx: &Fx) -> ExternalAdapter {
    ExternalAdapter::new(
        Arc::clone(&fx.service.store),
        Arc::clone(&fx.service.vault),
        fx.transport.clone(),
        Arc::clone(&fx.pool),
        fx.dir.path().join("tmp"),
        Arc::clone(&fx.service.cli),
    )
}

pub(super) fn request(asset: i64, thumbs: Vec<PathBuf>) -> ExternalRequest {
    ExternalRequest {
        asset_id: asset,
        media_type: "image".into(),
        kinds: vec![StepKind::Describe, StepKind::Keywords],
        language: "ko".into(),
        transfer: Transfer::default(),
        images: thumbs
            .into_iter()
            .map(|path| ImageInput { path, frame: None })
            .collect(),
    }
}

pub(super) fn subscription(fx: &Fx) {
    fx.service
        .set_provider_auth_mode("claude", "subscription")
        .unwrap();
}

pub(super) fn usage_rows(fx: &Fx) -> Vec<(String, bool, Option<String>)> {
    fx.pool
        .read(|c| {
            let mut s = c
                .prepare("SELECT kind, ok, auth_mode FROM ai_usage ORDER BY usage_id")
                .unwrap();
            Ok(s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect())
        })
        .unwrap()
}

// ── 설정·호환 ───────────────────────────────────────────────────────────

#[test]
fn t_aicli_set_01_settings_saved_before_auth_mode_existed_read_as_api_key() {
    // Arrange: 구 버전이 저장한 문서 (authMode 없음)
    let old = r#"{"settings":{"defaultProvider":"claude"},"providers":{"claude":{"model":"claude-x","lastCheck":null},"openai":{}}}"#;
    let doc: StoredAi = serde_json::from_str(old).unwrap();
    assert_eq!(
        doc.provider(mam_ai::external::Provider::Claude).auth_mode,
        mam_ai::external::AuthMode::ApiKey
    );

    // Act: 실제 DB 에 넣고 서비스를 다시 만든다
    let fx = Fx::new();
    fx.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES ('ai_settings', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [old],
            )?;
            Ok(())
        })
        .unwrap();
    let svc = fx.restart();

    // Assert
    for p in svc.list_providers() {
        assert_eq!(p.auth_mode.as_str(), "apiKey", "{:?}", p.provider);
        assert!(p.cli.is_none());
        assert_eq!(p.subscription_supported, p.provider.as_str() == "claude");
    }
    assert_eq!(fx.cli.runner.spawn_count(), 0, "탐지는 구독 모드일 때만");
}

#[test]
fn t_aicli_set_02_provider_status_serializes_the_contract_shape() {
    let fx = Fx::new();
    subscription(&fx);

    let v = serde_json::to_value(fx.service.list_providers()).unwrap();
    let claude = v
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == "claude")
        .unwrap();
    let openai = v
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["provider"] == "openai")
        .unwrap();

    assert_eq!(claude["authMode"], "subscription");
    assert_eq!(claude["subscriptionSupported"], true);
    assert_eq!(claude["cli"]["found"], true);
    assert_eq!(claude["cli"]["supported"], true);
    assert_eq!(claude["cli"]["version"], "2.1.318");
    assert_eq!(claude["cli"]["account"]["loggedIn"], true);
    assert_eq!(claude["cli"]["account"]["plan"], "max");
    assert_eq!(claude["cli"]["apiKeyEnv"], false);
    assert_eq!(openai["authMode"], "apiKey");
    assert_eq!(openai["subscriptionSupported"], false);
    assert!(openai["cli"].is_null());
}

#[test]
fn t_aicli_set_03_only_claude_may_use_subscription_and_modes_are_validated() {
    let fx = Fx::new();
    for p in ["openai", "gemini"] {
        let e = fx
            .service
            .set_provider_auth_mode(p, "subscription")
            .unwrap_err();
        assert!(matches!(e, AiFailure::Invalid(_)), "{p}");
    }
    assert!(matches!(
        fx.service
            .set_provider_auth_mode("claude", "oauth")
            .unwrap_err(),
        AiFailure::Invalid(_)
    ));
    assert!(matches!(
        fx.service
            .set_provider_auth_mode("nope", "apiKey")
            .unwrap_err(),
        AiFailure::Invalid(_)
    ));
    // API 키 모드로 되돌리는 것은 어느 공급자든 가능
    assert_eq!(
        fx.service
            .set_provider_auth_mode("openai", "apiKey")
            .unwrap()
            .auth_mode
            .as_str(),
        "apiKey"
    );
    assert_eq!(fx.cli.runner.spawn_count(), 0);
}

#[test]
fn t_aicli_set_04_mode_persists_across_restart_and_becomes_default_provider_when_none() {
    let fx = Fx::new();
    assert_eq!(fx.service.get_settings().default_provider, None);

    subscription(&fx);

    assert_eq!(
        fx.service
            .get_settings()
            .default_provider
            .map(|p| p.as_str()),
        Some("claude")
    );
    let svc = fx.restart();
    let claude = svc
        .list_providers()
        .into_iter()
        .find(|p| p.provider.as_str() == "claude")
        .unwrap();
    assert_eq!(claude.auth_mode.as_str(), "subscription");
    // 다시 API 키로 돌아가면 cli 는 null
    let back = svc.set_provider_auth_mode("claude", "apiKey").unwrap();
    assert!(back.cli.is_none());
}

#[test]
fn t_aicli_set_05_switching_mode_clears_the_previous_connection_check() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    subscription(&fx);
    fx.service.check_provider("claude").unwrap();
    assert!(
        fx.service
            .list_providers()
            .iter()
            .any(|p| p.last_check.is_some())
    );

    let st = fx
        .service
        .set_provider_auth_mode("claude", "apiKey")
        .unwrap();

    assert!(st.last_check.is_none());
}

// ── 탐지 ────────────────────────────────────────────────────────────────

#[test]
fn t_aicli_det_svc_01_detect_works_before_consent_and_never_sends_a_request() {
    let fx = Fx::new();
    subscription(&fx);

    let st = fx.service.detect_ai_cli("claude").unwrap();

    let cli = st.cli.unwrap();
    assert!(cli.found && cli.supported && cli.account.unwrap().logged_in);
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 0);
    assert_eq!(fx.transport.count(), 0);
    assert!(
        !fx.service.get_settings().consent.given,
        "동의 상태는 그대로"
    );
    assert!(matches!(
        fx.service.detect_ai_cli("openai").unwrap_err(),
        AiFailure::Invalid(_)
    ));
}

#[test]
fn t_aicli_det_svc_02_list_reuses_the_cache_but_detect_refreshes_it() {
    let fx = Fx::new();
    subscription(&fx); // 탐지 1회 (--version + auth status)
    let after_set = fx.cli.runner.spawn_count();
    fx.service.list_providers();
    fx.service.list_providers();
    assert_eq!(
        fx.cli.runner.spawn_count(),
        after_set,
        "목록 조회는 캐시를 쓴다"
    );

    // 로그아웃 상태로 바뀐 뒤 다시 확인
    fx.cli.scripts.lock().unwrap().status =
        vec![cli_line(r#"{"loggedIn":false}"#), Step::Exit(Some(1))];
    let stale = fx.service.list_providers();
    assert!(
        stale
            .iter()
            .find(|p| p.provider.as_str() == "claude")
            .unwrap()
            .cli
            .as_ref()
            .unwrap()
            .account
            .as_ref()
            .unwrap()
            .logged_in
    );
    let fresh = fx.service.detect_ai_cli("claude").unwrap();

    assert!(!fresh.cli.unwrap().account.unwrap().logged_in);
    assert!(fx.cli.runner.spawn_count() > after_set);
}

#[test]
fn t_aicli_det_svc_03_not_installed_and_api_key_env_are_reported() {
    let fx = Fx::new();
    fx.cli
        .env
        .installed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    fx.cli
        .env
        .vars
        .lock()
        .unwrap()
        .insert("ANTHROPIC_API_KEY".into());
    subscription(&fx);

    let cli = fx.service.detect_ai_cli("claude").unwrap().cli.unwrap();

    assert!(!cli.found && !cli.supported && cli.account.is_none());
    assert!(cli.api_key_env, "환경변수 존재만 표시 (값은 읽지 않는다)");
    assert_eq!(fx.cli.runner.spawn_count(), 0);
}

// ── 연결 확인 ───────────────────────────────────────────────────────────

#[test]
fn t_aicli_chk_01_check_needs_consent_and_spawns_nothing_without_it() {
    let fx = Fx::new();
    subscription(&fx);
    let before = fx.cli.runner.spawn_count();

    let e = fx.service.check_provider("claude").unwrap_err();

    assert!(matches!(e, AiFailure::ConsentRequired));
    assert_eq!(fx.cli.runner.spawn_count(), before);
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 0);
}

#[test]
fn t_aicli_chk_02_subscription_check_uses_the_cli_and_records_usage_with_mode() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    subscription(&fx);

    let r = fx.service.check_provider("claude").unwrap();

    assert!(r.ok && r.error_code.is_none());
    assert_eq!(fx.transport.count(), 0, "HTTP 요청 없음");
    assert_eq!(fx.cli.runner.count_with_arg("-p"), 1);
    assert_eq!(
        usage_rows(&fx),
        vec![("check".to_string(), true, Some("subscription".to_string()))]
    );
    let claude = fx
        .service
        .list_providers()
        .into_iter()
        .find(|p| p.provider.as_str() == "claude")
        .unwrap();
    assert!(claude.last_check.unwrap().ok);
}

#[test]
fn t_aicli_chk_03_check_failures_report_contract_codes() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    subscription(&fx);
    fx.cli.scripts.lock().unwrap().request = vec![
        cli_line(r#"{"type":"system","subtype":"init","apiKeySource":"none"}"#),
        cli_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"result":"Not logged in · Please run /login"}"#,
        ),
        Step::Exit(Some(1)),
    ];

    let r = fx.service.check_provider("claude").unwrap();

    assert!(!r.ok);
    assert_eq!(r.error_code.as_deref(), Some("cli_not_logged_in"));
    assert!(!usage_rows(&fx)[0].1);

    fx.cli
        .env
        .installed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let r = fx.service.check_provider("claude").unwrap();
    assert_eq!(r.error_code.as_deref(), Some("cli_not_found"));
}

#[test]
fn t_aicli_chk_04_check_counts_as_in_flight_while_it_runs() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    subscription(&fx);
    fx.cli.scripts.lock().unwrap().request = vec![Step::Hang];
    let svc = Arc::clone(&fx.service);
    let t = std::thread::spawn(move || svc.check_provider("claude"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while fx.cli.runner.count_with_arg("-p") == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let during = fx.service.in_flight().count();
    let refused = fx.service.cli_logout("claude");
    fx.service.terminate_cli(); // 멈춘 요청 정리
    let _ = t.join().unwrap();

    assert_eq!(during, 1);
    assert!(matches!(refused, Err(AiFailure::Busy)));
    assert_eq!(fx.service.in_flight().count(), 0);
}

// ── 분석 어댑터 ─────────────────────────────────────────────────────────

#[test]
fn t_aicli_ext_01_consent_off_means_zero_processes() {
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    subscription(&fx); // 탐지는 동의와 무관하게 허용된다
    let before = fx.cli.runner.spawn_count();
    let thumb = fx.write_thumb("t1.jpg");

    let e = adapter(&fx).analyze(&request(1, vec![thumb])).unwrap_err();

    assert_eq!(e.code, "consent_required");
    assert_eq!(
        fx.cli.runner.spawn_count(),
        before,
        "동의 전에는 어떤 프로세스도 띄우지 않는다"
    );
}

#[test]
fn t_aicli_ext_02_subscription_analyze_needs_no_key_and_sends_no_original_path() {
    // Arrange: API 키 없음, 동의함, 구독 모드
    let fx = Fx::new();
    fx.insert_asset(7, "image", "private/dir/holiday.jpg");
    fx.service.give_consent().unwrap();
    subscription(&fx);
    let thumb = fx.write_thumb("t7.jpg");

    // Act
    let out = adapter(&fx)
        .analyze(&request(7, vec![thumb.clone()]))
        .unwrap();

    // Assert
    assert_eq!(out.description, "붉은 노을");
    assert_eq!(out.provider, "claude");
    assert_eq!(out.usage.auth_mode.as_deref(), Some("subscription"));
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), (105, 20));
    assert_eq!(fx.transport.count(), 0, "HTTP 요청 없음");
    let cmd = fx
        .cli
        .runner
        .spawned
        .lock()
        .unwrap()
        .iter()
        .find(|c| c.args.iter().any(|a| a == "-p"))
        .cloned()
        .unwrap();
    let args: String = cmd
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    let stdin = String::from_utf8(cmd.stdin.unwrap()).unwrap();
    let cwd = cmd.cwd.unwrap();
    for hay in [args.as_str(), stdin.as_str()] {
        assert!(!hay.contains("private/dir") && !hay.contains(r"private\dir"));
        assert!(
            !hay.contains(&thumb.to_string_lossy().into_owned()),
            "썸네일 경로도 인자·입력에 없다"
        );
    }
    assert!(stdin.contains("holiday.jpg"), "정제된 파일 이름만");
    // 사용자 컨텍스트 격리: 작업 폴더는 OS 임시 폴더 아래이고 MAMData(데이터 루트) 밖이다, 인자에는 두 격리 플래그가 있다
    assert!(
        cwd.starts_with(std::env::temp_dir().join("mam-ai-cli")),
        "{cwd:?}"
    );
    assert!(!cwd.starts_with(fx.dir.path()), "MAMData 밖: {cwd:?}");
    assert!(args.contains("--safe-mode") && args.contains("--system-prompt"));
    assert!(!cwd.exists(), "작업 폴더는 끝난 뒤 지워진다");
    assert_eq!(adapter(&fx).auth_mode().as_deref(), Some("subscription"));
}

#[test]
fn t_aicli_ext_03_cli_failures_become_step_failures_with_contract_codes() {
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service.give_consent().unwrap();
    subscription(&fx);
    let thumb = fx.write_thumb("t1.jpg");
    let a = adapter(&fx);
    fx.cli.scripts.lock().unwrap().request = vec![
        cli_line(r#"{"type":"system","subtype":"init","apiKeySource":"none"}"#),
        cli_line(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1}}"#,
        ),
        cli_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"x"}"#,
        ),
        Step::Exit(Some(1)),
    ];

    let e = a.analyze(&request(1, vec![thumb.clone()])).unwrap_err();

    assert_eq!(e.code, "subscription_limit");
    assert!(
        e.retry_after_secs == Some(60),
        "한도 거부는 재시도 대기를 준다"
    );

    // 한도 일시정지는 설치 확인보다 먼저다 (프로세스도 탐지도 하지 않는다) - 여기서는 해제하고 설치 실패를 본다
    fx.service.cli.clear_pause();
    fx.cli
        .env
        .installed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let e = a.analyze(&request(1, vec![thumb])).unwrap_err();
    assert_eq!(e.code, "cli_not_found");
}

#[test]
fn t_aicli_ext_04_subscription_stored_for_other_providers_is_skipped_not_spawned() {
    // 손으로 편집한 설정: openai 에 subscription 이 저장돼 있다 -> 요청 없이 건너뜀
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service.give_consent().unwrap();
    fx.service
        .update_settings(json!({"defaultProvider": "openai"}))
        .unwrap();
    fx.service
        .store
        .update(|d| {
            d.provider_mut(parse_provider("openai").unwrap()).auth_mode =
                mam_ai::external::AuthMode::Subscription;
            Ok(())
        })
        .unwrap();
    let thumb = fx.write_thumb("t1.jpg");

    let e = adapter(&fx).analyze(&request(1, vec![thumb])).unwrap_err();

    assert_eq!(e.code, "disabled");
    assert_eq!(fx.cli.runner.spawn_count(), 0);
    assert_eq!(fx.transport.count(), 0);
    // 서비스 쪽도 API 키 모드로 해석한다
    let openai = fx
        .service
        .list_providers()
        .into_iter()
        .find(|p| p.provider.as_str() == "openai")
        .unwrap();
    assert_eq!(openai.auth_mode.as_str(), "apiKey");
}

#[test]
fn t_aicli_ext_05_api_key_mode_is_unchanged() {
    // 회귀: API 키 모드는 HTTP 로 요청하고 CLI 를 띄우지 않으며 사용량 모드는 apiKey
    let fx = Fx::new();
    fx.insert_asset(7, "image", "x/holiday.jpg");
    fx.consent_and_key();
    let thumb = fx.write_thumb("t7.jpg");

    let out = adapter(&fx).analyze(&request(7, vec![thumb])).unwrap();

    assert_eq!(out.provider, "openai");
    assert_eq!(out.usage.auth_mode.as_deref(), Some("apiKey"));
    assert_eq!(fx.transport.count(), 1);
    assert_eq!(fx.cli.runner.spawn_count(), 0);
}

// ── 자동·예약 ───────────────────────────────────────────────────────────

#[test]
fn t_aicli_sched_01_background_sweep_is_skipped_in_subscription_mode_but_new_assets_stay() {
    // Arrange: 구독 모드 + 동의 + 자동/백그라운드 켬. 키는 없다.
    let fx = Fx::with(StubTransport::new(200, "{}"), 1_800_025_200_000);
    fx.insert_asset(1, "image", "old.jpg");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0); // 워터마크 시작
    fx.service.give_consent().unwrap();
    subscription(&fx);
    fx.service
        .update_settings(json!({"autoAnalyzeNew": true, "backgroundAnalyze": true}))
        .unwrap();

    // Act + Assert 1: 일괄 스윕은 건너뛴다 (API 키 모드였다면 기존 자산 1건이 등록된다)
    let r = fx.service.tick_background().unwrap();
    assert_eq!((r.queued, r.skipped), (0, 0));

    // Assert 2: 신규 자산 자동 분석은 유지된다 - 키가 없어도 외부 단계가 등록된다
    fx.insert_asset(2, "image", "new.jpg");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 1);
    let st = fx.service.get_asset_ai(2).unwrap().status;
    assert!(
        st.iter()
            .any(|s| s.kind.is_external() && s.state == "queued")
    );
}

#[test]
fn t_aicli_sched_02_api_key_mode_background_sweep_still_runs() {
    // 회귀 대조: 같은 조건에서 API 키 모드는 스윕이 등록한다
    let fx = Fx::with(StubTransport::new(200, "{}"), 1_800_025_200_000);
    fx.consent_and_key();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service
        .update_settings(json!({"backgroundAnalyze": true}))
        .unwrap();

    assert_eq!(fx.service.tick_background().unwrap().queued, 1);
}

// ── 로그인·로그아웃 ─────────────────────────────────────────────────────

#[test]
fn t_aicli_login_svc_01_login_flow_opens_the_captured_url_only() {
    let fx = Fx::new();
    fx.cli.scripts.lock().unwrap().login = vec![cli_line(LOGIN_URL), Step::Hang];

    let st = fx.service.cli_login_start("claude").unwrap();

    assert_eq!(
        st,
        LoginState::Waiting {
            url: Some(LOGIN_URL.into())
        }
    );
    assert_eq!(
        *fx.cli.opener.0.lock().unwrap(),
        vec![LOGIN_URL.to_string()]
    );
    assert!(fx.service.open_cli_login_url("claude").unwrap());
    assert_eq!(fx.cli.opener.0.lock().unwrap().len(), 2);
    assert_eq!(
        fx.service.cli_login_status("claude").unwrap(),
        LoginState::Waiting {
            url: Some(LOGIN_URL.into())
        }
    );
    assert_eq!(
        fx.service.cli_login_cancel("claude").unwrap(),
        LoginState::None
    );
    assert!(!fx.service.open_cli_login_url("claude").unwrap());
    assert_eq!(fx.cli.runner.killed_pids().len(), 1);
}

#[test]
fn t_aicli_login_svc_02_login_commands_accept_only_claude() {
    let fx = Fx::new();
    for p in ["openai", "gemini", "nope"] {
        assert!(
            matches!(fx.service.cli_login_start(p), Err(AiFailure::Invalid(_))),
            "{p}"
        );
        assert!(matches!(
            fx.service.cli_login_status(p),
            Err(AiFailure::Invalid(_))
        ));
        assert!(matches!(
            fx.service.cli_login_cancel(p),
            Err(AiFailure::Invalid(_))
        ));
        assert!(matches!(
            fx.service.open_cli_login_url(p),
            Err(AiFailure::Invalid(_))
        ));
        assert!(matches!(
            fx.service.cli_logout(p),
            Err(AiFailure::Invalid(_))
        ));
    }
    assert_eq!(fx.cli.runner.spawn_count(), 0);
}

#[test]
fn t_aicli_login_svc_03_finished_login_is_delivered_once_and_refreshes_the_cli_status() {
    let fx = Fx::new();
    subscription(&fx);
    // 로그인 전: 계정 없음
    fx.cli.scripts.lock().unwrap().status =
        vec![cli_line(r#"{"loggedIn":false}"#), Step::Exit(Some(1))];
    assert!(
        !fx.service
            .detect_ai_cli("claude")
            .unwrap()
            .cli
            .unwrap()
            .account
            .unwrap()
            .logged_in
    );
    // 로그인 명령은 URL 을 내고 바로 종료한다(승인 완료 모사) — 이제 상태 명령은 로그인됨
    {
        let mut s = fx.cli.scripts.lock().unwrap();
        s.login = vec![cli_line(LOGIN_URL), Step::Exit(Some(0))];
        s.status = vec![
            cli_line(r#"{"loggedIn":true,"subscriptionType":"max"}"#),
            Step::Exit(Some(0)),
        ];
    }
    fx.service.cli_login_start("claude").unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let done = loop {
        let s = fx.service.cli_login_status("claude").unwrap();
        if !matches!(s, LoginState::Waiting { .. }) || std::time::Instant::now() > deadline {
            break s;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };

    assert_eq!(done, LoginState::Done { verified: true });
    assert_eq!(
        fx.service.cli_login_status("claude").unwrap(),
        LoginState::None
    );
    let claude = fx
        .service
        .list_providers()
        .into_iter()
        .find(|p| p.provider.as_str() == "claude")
        .unwrap();
    assert!(
        claude.cli.unwrap().account.unwrap().logged_in,
        "완료 뒤 캐시가 갱신된다"
    );
}

#[test]
fn t_aicli_logout_svc_01_refused_with_ai_busy_while_a_request_is_in_flight() {
    let fx = Fx::new();
    subscription(&fx);
    let guard = fx.service.in_flight().begin();

    let e = fx.service.cli_logout("claude").unwrap_err();

    assert!(matches!(e, AiFailure::Busy));
    assert_eq!(
        fx.cli.runner.count_with_arg("logout"),
        0,
        "로그아웃 명령은 실행되지 않았다"
    );
    let ce: crate::error::CommandError = e.into();
    assert_eq!(ce.code(), crate::error::ErrorCode::AiBusy);
    drop(guard);

    // 요청이 끝나면 로그아웃된다 - 상태 명령이 로그아웃을 보고하도록 바꿔 둔다
    fx.cli.scripts.lock().unwrap().status =
        vec![cli_line(r#"{"loggedIn":false}"#), Step::Exit(Some(1))];
    let st = fx.service.cli_logout("claude").unwrap();
    assert_eq!(fx.cli.runner.count_with_arg("logout"), 1);
    assert!(!st.cli.unwrap().account.unwrap().logged_in);
}

#[test]
fn t_aicli_logout_svc_02_cli_failures_map_to_fixed_provider_errors() {
    let fx = Fx::new();
    subscription(&fx);
    fx.cli.scripts.lock().unwrap().logout = vec![Step::Exit(Some(1))];
    let e: crate::error::CommandError = fx.service.cli_logout("claude").unwrap_err().into();
    assert_eq!(e.code(), crate::error::ErrorCode::ProviderError);

    fx.cli
        .env
        .installed
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        fx.service.cli_logout("claude"),
        Err(AiFailure::Cli("cli_not_found"))
    ));
}

#[test]
fn t_aicli_term_01_terminate_cli_kills_login_and_blocks_new_runs() {
    let fx = Fx::new();
    fx.cli.scripts.lock().unwrap().login = vec![cli_line(LOGIN_URL), Step::Hang];
    fx.service.cli_login_start("claude").unwrap();

    fx.service.terminate_cli();
    fx.service.cli_login_cancel("claude").unwrap();

    assert!(!fx.cli.runner.killed_pids().is_empty());
    let st = fx.service.cli_login_start("claude").unwrap();
    assert_eq!(
        st,
        LoginState::Failed {
            error_code: "cli_not_found".into()
        },
        "종료 뒤에는 새 프로세스를 띄우지 않는다"
    );
}
