//! 공급자 어댑터 — 요청 형식을 기록된 픽스처로 고정하고 응답/토큰 파싱을 검증한다. 분류: Happy / External / Validation.
//! 픽스처 갱신: `UPDATE_FIXTURES=1 cargo test -p mam-ai --test external_adapters` (공식 문서 대조 후 커밋).

#[path = "external_support.rs"]
mod support;

use base64::Engine;
use mam_ai::external::{AiClient, ConsentGate, ExternalError, Provider, prepare_image};
use support::*;

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

/// 전송될 JPEG 의 base64 (재인코딩이 결정적이므로 요청에 들어간 값과 같다).
fn expected_b64(png_bytes: &[u8], edge: u32) -> String {
    let (jpeg, _) = prepare_image(png_bytes, edge).unwrap();
    base64::engine::general_purpose::STANDARD.encode(jpeg)
}

fn run(
    provider: Provider,
    ok_body: String,
) -> (Rig, mam_ai::external::AnalyzeOutput, serde_json::Value) {
    let img = png(320, 200);
    let r = rig(provider, vec![resp(200, &[], &ok_body)], true);
    let out = r
        .client
        .analyze(&gate(), &request(vec![img.clone()]))
        .unwrap();
    let sent = r.transport.requests.lock().unwrap()[0].clone();
    let norm = normalize(&sent, &expected_b64(&img, 1024));
    (r, out, norm)
}

fn assert_output(
    out: &mam_ai::external::AnalyzeOutput,
    provider: Provider,
    model: &str,
    tokens: (u64, u64),
) {
    assert_eq!(out.description, "해변 풍경");
    assert_eq!(out.keywords, vec!["해변", "바다"]);
    assert_eq!(
        (out.summary.as_str(), out.language.as_str()),
        ("여름 해변", "ko")
    );
    assert_eq!((out.provider, out.model.as_str()), (provider, model));
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), tokens);
    assert!(out.usage.ok && out.usage.error_code.is_none());
}

#[test]
fn t_ext_09_openai_request_matches_fixture_and_response_parsed() {
    let (r, out, norm) = run(Provider::OpenAi, openai_ok(MODEL_JSON));
    assert_fixture("openai_request.json", &norm);
    assert_output(&out, Provider::OpenAi, "gpt-4o-mini", (120, 30));
    assert_eq!(
        r.transport.requests.lock().unwrap()[0].header("authorization"),
        Some(format!("Bearer {SECRET}").as_str())
    );
}

#[test]
fn t_ext_10_gemini_request_matches_fixture_key_not_in_url() {
    let (_, out, norm) = run(Provider::Gemini, gemini_ok(MODEL_JSON));
    assert_fixture("gemini_request.json", &norm);
    assert_output(&out, Provider::Gemini, "gemini-2.0-flash", (200, 40));
    assert!(!norm["url"].as_str().unwrap().contains('?'));
}

#[test]
fn t_ext_11_claude_request_and_headers_match_fixture_response_parsed() {
    let (_, out, norm) = run(Provider::Claude, claude_ok(MODEL_JSON));
    assert_fixture("claude_request.json", &norm);
    assert_output(&out, Provider::Claude, "claude-haiku-4-5", (300, 50));
    assert_eq!(norm["headers"]["anthropic-version"], "2023-06-01");
}

#[test]
fn t_ext_11_tolerant_parsing_fences_extra_text_and_keyword_string() {
    let fenced = format!("Here you go:\n```json\n{MODEL_JSON}\n```\nHope it helps!");
    let (_, out, _) = run(Provider::Claude, claude_ok(&fenced));
    assert_eq!(out.keywords.len(), 2);
    let csv = r#"{"description":"d","keywords":"a, b ,,c","ocrText":"TEXT"}"#;
    let (_, out, _) = run(Provider::OpenAi, openai_ok(csv));
    assert_eq!(out.keywords, vec!["a", "b", "c"]);
    assert_eq!(
        (out.ocr_text.as_str(), out.language.as_str()),
        ("TEXT", "ko")
    ); // 언어는 요청 값으로 대체
}

#[test]
fn t_ext_11_invalid_json_or_missing_text_is_bad_response() {
    let cases = [
        (
            Provider::Claude,
            claude_ok("sorry, I cannot help with that"),
        ),
        (Provider::Claude, claude_ok("{ broken json")),
        (Provider::OpenAi, r#"{"choices":[]}"#.to_string()),
        (
            Provider::Gemini,
            r#"{"candidates":[{"content":{"parts":[]}}]}"#.to_string(),
        ),
        (Provider::Gemini, "<html>gateway</html>".to_string()),
    ];
    for (p, body) in cases {
        let r = rig(p, vec![resp(200, &[], &body)], true);
        let e = r
            .client
            .analyze(&gate(), &request(vec![png(64, 64)]))
            .unwrap_err();
        assert!(matches!(e, ExternalError::BadResponse(_)), "{p:?}: {e:?}");
        assert_eq!(e.ui_code(), "provider_error");
        assert_eq!(r.transport.calls(), 1); // 응답 해석 실패는 재시도하지 않는다
        let u = r.sink.0.lock().unwrap().clone();
        assert!(u.len() == 1 && !u[0].ok && u[0].error_code.as_deref() == Some("bad_response"));
    }
}

#[test]
fn t_ext_09_custom_model_is_used_and_missing_usage_defaults_to_zero() {
    let r = rig(
        Provider::OpenAi,
        vec![resp(
            200,
            &[],
            r#"{"choices":[{"message":{"content":"{}"}}]}"#,
        )],
        true,
    );
    let mut req = request(vec![png(64, 64)]);
    req.model = Some("gpt-custom-1".into());
    let out = r.client.analyze(&gate(), &req).unwrap();
    assert_eq!(
        (
            out.model.as_str(),
            out.usage.input_tokens,
            out.usage.output_tokens
        ),
        ("gpt-custom-1", 0, 0)
    );
    assert!(
        String::from_utf8(r.transport.requests.lock().unwrap()[0].body.clone())
            .unwrap()
            .contains("gpt-custom-1")
    );
}
