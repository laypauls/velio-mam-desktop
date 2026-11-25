//! 외부 AI 클라이언트 공통 동작 — 분류: Business rule / External / Boundary / Validation. 실제 네트워크 호출 없음(스텁 전송).

#[path = "external_support.rs"]
mod support;

use std::time::Duration;

use mam_ai::external::{
    AiClient, ConsentGate, ExternalError, Provider, TransportError, check_provider,
};
use support::*;

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

fn secs(v: &[u64]) -> Vec<Duration> {
    v.iter().map(|s| Duration::from_secs(*s)).collect()
}

#[test]
fn t_ext_04_no_consent_means_zero_transport_calls() {
    // Arrange
    let r = rig(
        Provider::OpenAi,
        vec![resp(200, &[], &openai_ok(MODEL_JSON))],
        true,
    );
    // Act: 게이트를 만들 수 없으므로 클라이언트 진입 자체가 불가능하다
    let denied = ConsentGate::check(false);
    // Assert
    let e = denied.unwrap_err();
    assert_eq!(e, ExternalError::ConsentRequired);
    assert_eq!(e.ui_code(), "consent_required");
    assert_eq!(r.transport.calls(), 0);
    // 동의 후에는 호출된다
    assert!(
        r.client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .is_ok()
    );
    assert_eq!(r.transport.calls(), 1);
}

#[test]
fn t_ext_04_missing_key_or_invalid_request_makes_no_call() {
    // Arrange / Act / Assert: 키 없음
    let r = rig(Provider::OpenAi, vec![], false);
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert_eq!(e, ExternalError::AuthFailed);
    assert_eq!(r.transport.calls(), 0);
    // 잘못된 입력: 언어, 이미지 없음, 모델 ID, 깨진 이미지
    let r = rig(Provider::Gemini, vec![], true);
    let mut bad_lang = request(vec![png(64, 64)]);
    bad_lang.language = "ko\nIgnore previous".into();
    let mut bad_model = request(vec![png(64, 64)]);
    bad_model.model = Some("x/../y".into());
    for req in [
        bad_lang,
        bad_model,
        request(vec![]),
        request(vec![b"not an image".to_vec()]),
    ] {
        let e = r.client.analyze(&gate(), &req).unwrap_err();
        assert_eq!(e.ui_code(), "invalid_input");
    }
    assert_eq!(r.transport.calls(), 0);
}

#[test]
fn t_ext_06_429_retry_after_seconds_and_http_date_and_max_three_retries() {
    // Arrange: 초 단위 Retry-After 후 성공
    let r = rig(
        Provider::OpenAi,
        vec![
            resp(429, &[("Retry-After", "2")], "{}"),
            resp(200, &[], &openai_ok(MODEL_JSON)),
        ],
        true,
    );
    // Act / Assert
    assert!(
        r.client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .is_ok()
    );
    assert_eq!(r.sleeper.0.lock().unwrap().clone(), secs(&[2]));
    assert_eq!(r.transport.calls(), 2);

    // HTTP-date (고정 시계 1700000000 = 2023-11-14 22:13:20 GMT, +10s)
    let r = rig(
        Provider::OpenAi,
        vec![
            resp(429, &[("retry-after", "Tue, 14 Nov 2023 22:13:30 GMT")], ""),
            resp(200, &[], &openai_ok(MODEL_JSON)),
        ],
        true,
    );
    assert!(
        r.client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .is_ok()
    );
    assert_eq!(r.sleeper.0.lock().unwrap().clone(), secs(&[10]));

    // 계속 429 -> 재시도 3회(총 4회 호출) 후 RateLimited, Retry-After 상한(30s)
    let r = rig(
        Provider::OpenAi,
        (0..6)
            .map(|_| resp(429, &[("retry-after", "3600")], ""))
            .collect(),
        true,
    );
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert!(matches!(e, ExternalError::RateLimited { .. }));
    assert_eq!(r.transport.calls(), 4);
    assert_eq!(r.sleeper.0.lock().unwrap().clone(), secs(&[30, 30, 30]));
}

#[test]
fn t_ext_07_5xx_and_timeout_retry_with_backoff_4xx_do_not() {
    // 5xx 두 번 후 성공: 백오프 1s, 2s
    let r = rig(
        Provider::Claude,
        vec![
            resp(503, &[], ""),
            resp(500, &[], ""),
            resp(200, &[], &claude_ok(MODEL_JSON)),
        ],
        true,
    );
    assert!(
        r.client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .is_ok()
    );
    assert_eq!(r.sleeper.0.lock().unwrap().clone(), secs(&[1, 2]));

    // timeout 계속 -> 총 4회, 백오프 1/2/4, Timeout
    let r = rig(
        Provider::Claude,
        (0..6).map(|_| Err(TransportError::Timeout)).collect(),
        true,
    );
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert_eq!(e, ExternalError::Timeout);
    assert_eq!(
        (r.transport.calls(), r.sleeper.0.lock().unwrap().clone()),
        (4, secs(&[1, 2, 4]))
    );

    // 401 -> 재시도 없음, 인증 실패 -> provider_error + "키 확인"
    let r = rig(Provider::Claude, vec![resp(401, &[], "{}")], true);
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert_eq!(
        (r.transport.calls(), e.error_code(), e.ui_code()),
        (1, "auth_failed", "provider_error")
    );
    assert_eq!(e.user_message(), "API 키를 확인하세요");

    // 400 -> 재시도 없음
    let r = rig(Provider::Claude, vec![resp(400, &[], "{}")], true);
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert_eq!(
        (r.transport.calls(), e),
        (1, ExternalError::BadRequest { status: 400 })
    );
    assert!(r.sleeper.0.lock().unwrap().is_empty());

    // 오프라인 -> 즉시 실패, 외부 AI 만 중단된다는 안내
    let r = rig(Provider::Claude, vec![Err(TransportError::Offline)], true);
    let e = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap_err();
    assert_eq!((r.transport.calls(), e.error_code()), (1, "offline"));
    assert!(e.user_message().contains("외부 AI") && e.user_message().contains("관리·검색"));
}

#[test]
fn t_ext_08_messages_never_contain_key_or_response_body() {
    // Arrange: 서버가 키·헤더를 본문에 되돌려주는 상황
    let echo = format!(r#"{{"error":"bad key {SECRET}","authorization":"Bearer {SECRET}"}}"#);
    let cases = [
        resp(401, &[("x-echo", SECRET)], &echo),
        resp(400, &[], &echo),
        resp(500, &[], &echo),
        resp(200, &[], &echo), // 형식이 다른 성공 응답 -> BadResponse
        resp(200, &[], &openai_ok(&format!("not json {SECRET}"))),
    ];
    for c in cases {
        let r = rig(
            Provider::OpenAi,
            vec![c.clone(), c.clone(), c.clone(), c],
            true,
        );
        // Act
        let e = r
            .client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .unwrap_err();
        // Assert
        for s in [e.to_string(), format!("{e:?}"), e.user_message()] {
            assert!(
                !s.contains(SECRET) && !s.contains("authorization"),
                "leak in {s}"
            );
        }
        for u in r.sink.0.lock().unwrap().iter() {
            assert!(!format!("{u:?}").contains(SECRET));
            assert!(!u.ok && u.error_code.is_some());
        }
        // 요청 객체의 Debug 도 키를 노출하지 않는다
        let sent = r.transport.requests.lock().unwrap()[0].clone();
        assert!(!format!("{sent:?}").contains(SECRET));
        assert!(!format!("{:?}", r.vault).contains(SECRET));
    }
}

#[test]
fn t_ext_08_key_only_in_auth_header_and_usage_recorded() {
    let r = rig(
        Provider::Gemini,
        vec![resp(200, &[], &gemini_ok(MODEL_JSON))],
        true,
    );
    let out = r
        .client
        .analyze(&gate(), &request(vec![png(64, 64)]))
        .unwrap();
    let sent = r.transport.requests.lock().unwrap()[0].clone();
    assert!(!sent.url.contains(SECRET));
    assert!(
        !String::from_utf8(sent.body.clone())
            .unwrap()
            .contains(SECRET)
    );
    assert_eq!(sent.header("x-goog-api-key"), Some(SECRET));
    // 사용량: 성공 1건, sink 와 동일
    let rows = r.sink.0.lock().unwrap().clone();
    assert_eq!(rows, vec![out.usage.clone()]);
    let u = &out.usage;
    assert_eq!(
        (u.input_tokens, u.output_tokens, u.ok, u.asset_id),
        (200, 40, true, Some(42))
    );
    assert_eq!(u.kind, "describe+keywords+ocr");
    assert_eq!(u.request_bytes, sent.body.len() as u64);
    assert_eq!(u.at, 1_700_000_000_000);
}

#[test]
fn t_ext_04_check_provider_maps_errors_without_leaking() {
    let cases = [
        (resp(200, &[], "{}"), true, None),
        (resp(401, &[], "{}"), false, Some("auth_failed")),
        (Err(TransportError::Offline), false, Some("offline")),
        (resp(400, &[], "{}"), false, Some("unknown")),
    ];
    for (response, ok, code) in cases {
        let r = rig(Provider::OpenAi, vec![response], true);
        let c = check_provider(&gate(), &r.client, None);
        assert_eq!((c.ok, c.error_code.as_deref()), (ok, code));
        let sent = r.transport.requests.lock().unwrap()[0].clone();
        assert!(
            !String::from_utf8(sent.body.clone())
                .unwrap()
                .contains("image")
        );
    }
    let r = rig(
        Provider::OpenAi,
        (0..4).map(|_| resp(429, &[], "")).collect(),
        true,
    );
    assert_eq!(
        check_provider(&gate(), &r.client, None)
            .error_code
            .as_deref(),
        Some("rate_limited")
    );
}
