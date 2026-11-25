//! stream-json 해석 시험 — 합성 픽스처(실측 줄의 모양만 따랐고 경로·세션 ID 는 없다).
//! 분류: Happy / Boundary / Validation / External(오프라인·한도·로그인) / Business rule(API 키 과금 표시)

use super::*;

pub(crate) const INIT_SUB: &str = r#"{"type":"system","subtype":"init","apiKeySource":"none","model":"claude-haiku-5-5","tools":["StructuredOutput"]}"#;
pub(crate) const INIT_KEY: &str = r#"{"type":"system","subtype":"init","apiKeySource":"ANTHROPIC_API_KEY","model":"claude-haiku-5-5","tools":[]}"#;
pub(crate) const RETRY_NULL: &str = r#"{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":503,"error_status":null,"error":"unknown"}"#;
pub(crate) const RETRY_503: &str = r#"{"type":"system","subtype":"api_retry","attempt":1,"max_retries":10,"retry_delay_ms":503,"error_status":503,"error":"server_error"}"#;
pub(crate) const OK_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"result":"{\"description\":\"d\"}","structured_output":{"description":"설명","keywords":["a","b"],"ocrText":"","summary":"요약","language":"ko"},"usage":{"input_tokens":2,"cache_creation_input_tokens":7187,"cache_read_input_tokens":10,"output_tokens":1317}}"#;

fn feed_all(lines: &[&str]) -> (StreamState, Vec<Flow>) {
    let mut st = StreamState::default();
    let flows = lines.iter().map(|l| st.feed(l)).collect();
    (st, flows)
}

#[test]
fn t_aicli_stream_01_success_result_with_structured_output_and_tokens() {
    // Arrange + Act
    let (st, flows) = feed_all(&[INIT_SUB, r#"{"type":"assistant","message":{}}"#, OK_RESULT]);

    // Assert
    assert!(flows.iter().all(|f| *f == Flow::Continue));
    assert_eq!(st.model.as_deref(), Some("claude-haiku-5-5"));
    assert!(!st.billed_to_api_key());
    let r = st.result.unwrap();
    assert!(!r.is_error);
    assert_eq!(r.input_tokens, 2 + 7187 + 10);
    assert_eq!(r.output_tokens, 1317);
    assert_eq!(r.structured.unwrap()["language"], "ko");
}

#[test]
fn t_aicli_stream_02_unknown_and_malformed_lines_are_ignored() {
    let (st, flows) = feed_all(&[
        "not json at all",
        "",
        r#"{"type":"future_event","x":1}"#,
        r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":5}"#,
        OK_RESULT,
    ]);
    assert!(flows.iter().all(|f| *f == Flow::Continue));
    assert!(st.result.is_some());
}

#[test]
fn t_aicli_stream_03_two_consecutive_null_status_retries_stop_as_offline() {
    let (st, flows) = feed_all(&[INIT_SUB, RETRY_NULL, RETRY_NULL]);
    assert_eq!(flows, vec![Flow::Continue, Flow::Continue, Flow::Stop]);
    assert!(st.offline);
}

#[test]
fn t_aicli_stream_04_single_retry_or_retry_with_http_status_is_not_offline() {
    // 경계: 1회, 상태 코드가 있는 재시도(서버 응답은 받았다), 중간에 응답이 오면 계수 초기화
    let (st, flows) = feed_all(&[RETRY_NULL]);
    assert!(!st.offline && flows == vec![Flow::Continue]);
    let (st, _) = feed_all(&[RETRY_NULL, RETRY_503, RETRY_NULL]);
    assert!(!st.offline);
    let (st, _) = feed_all(&[RETRY_NULL, r#"{"type":"assistant"}"#, RETRY_NULL]);
    assert!(!st.offline);
}

#[test]
fn t_aicli_stream_05_rejected_rate_limit_event_records_reset_time() {
    let (st, _) = feed_all(&[
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":5,"overageStatus":"rejected"}}"#,
    ]);
    assert!(!st.limit_rejected, "overage 거부는 한도 거부가 아니다");
    let (st, _) = feed_all(&[
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1794340800}}"#,
    ]);
    assert!(st.limit_rejected);
    assert_eq!(st.limit_resets_at, Some(1_794_340_800));
}

#[test]
fn t_aicli_stream_06_api_key_source_is_reported_from_init() {
    let (st, _) = feed_all(&[INIT_KEY]);
    assert!(st.billed_to_api_key());
    let (st, _) = feed_all(&[r#"{"type":"result","is_error":false}"#]);
    assert!(!st.billed_to_api_key(), "init 을 못 봤으면 단정하지 않는다");
}

fn err_of(lines: &[&str]) -> ExternalError {
    let (st, _) = feed_all(lines);
    let r = st.result.clone().expect("result line");
    map_error_result(&r, &st, Some(120))
}

#[test]
fn t_aicli_stream_07_maps_login_and_auth_errors() {
    let not_logged = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"result":"Not logged in · Please run /login"}"#;
    assert_eq!(
        err_of(&[INIT_SUB, not_logged]),
        ExternalError::CliNotLoggedIn
    );
    let r401 = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"x"}"#;
    assert_eq!(err_of(&[INIT_SUB, r401]), ExternalError::CliNotLoggedIn);
    // 환경변수 API 키로 실행 중이면 같은 401 이 키 문제다 (실측: 가짜 키 → Invalid API key)
    assert_eq!(err_of(&[INIT_KEY, r401]), ExternalError::AuthFailed);
}

#[test]
fn t_aicli_stream_08_maps_limit_errors_with_retry_after() {
    let r429 = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"x"}"#;
    assert_eq!(
        err_of(&[INIT_SUB, r429]),
        ExternalError::SubscriptionLimit {
            retry_after_secs: Some(120)
        }
    );
    let rejected =
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":9}}"#;
    let generic = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"result":"Claude AI usage limit reached"}"#;
    assert!(matches!(
        err_of(&[INIT_SUB, rejected, generic]),
        ExternalError::SubscriptionLimit { .. }
    ));
}

#[test]
fn t_aicli_stream_09_maps_connection_failure_server_and_unknown_errors() {
    let conn = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"api_error":"connection_failed","result":"API Error: Connection refused (ECONNREFUSED)"}"#;
    assert_eq!(err_of(&[INIT_SUB, conn]), ExternalError::Offline);
    let s500 = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":500,"result":"x"}"#;
    assert_eq!(
        err_of(&[INIT_SUB, s500]),
        ExternalError::Server { status: 500 }
    );
    let s400 = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":400,"result":"x"}"#;
    assert_eq!(
        err_of(&[INIT_SUB, s400]),
        ExternalError::BadRequest { status: 400 }
    );
    let weird = r#"{"type":"result","subtype":"error_during_execution","result":"???"}"#;
    assert!(matches!(
        err_of(&[INIT_SUB, weird]),
        ExternalError::BadResponse(_)
    ));
}

#[test]
fn t_aicli_stream_10_error_text_never_reaches_the_error_message() {
    let secret = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"result":"boom C:\\Users\\me\\secret.txt sk-ant-XYZ"}"#;
    let e = err_of(&[INIT_SUB, secret]);
    let shown = format!("{e} {e:?} {}", e.user_message());
    assert!(!shown.contains("secret") && !shown.contains("sk-ant"));
}

#[test]
fn t_aicli_stream_11_stderr_login_hint() {
    assert!(stderr_says_not_logged_in(
        "Error: Not logged in. Run claude auth login"
    ));
    assert!(!stderr_says_not_logged_in("segfault"));
}

#[test]
fn t_aicli_stream_20_429_on_an_env_api_key_is_a_rate_limit_not_a_subscription_limit() {
    // Arrange: 구독을 멈추면 안 되는 경우 - 요청이 환경변수 API 키로 과금됐다
    let r429 = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"x"}"#;

    // Act + Assert
    assert_eq!(
        err_of(&[INIT_KEY, r429]),
        ExternalError::RateLimited {
            retry_after_secs: Some(120)
        }
    );
    // 회귀 대조: 구독 요청의 429 는 그대로 구독 한도
    assert!(matches!(
        err_of(&[INIT_SUB, r429]),
        ExternalError::SubscriptionLimit { .. }
    ));
}
