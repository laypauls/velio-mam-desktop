//! T-aij-auth-* - 외부 AI 사용량의 인증 방식(`ai_usage.auth_mode`) 기록·집계.
//! 분류: Happy / Business rule(실제 과금 방식 우선) / Boundary(기존 NULL 행) / External(실패 경로)

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::StepFailure;
use mam_core::ai_store::{UsageEntry, list_usage, record_usage};

fn modes(e: &AiEnv) -> Vec<(bool, Option<String>)> {
    e.env.read(|c| {
        let mut s = c
            .prepare("SELECT ok, auth_mode FROM ai_usage ORDER BY usage_id")
            .unwrap();
        s.query_map([], |r| Ok((r.get::<_, bool>(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

fn run_one(e: &AiEnv) {
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
    let _ = e.run_next(&|| false);
}

#[test]
fn t_aij_auth_01_success_usage_takes_the_configured_mode_when_the_result_has_none() {
    let e = AiEnv::new();
    *e.ext.auth_mode.lock().unwrap() = Some("subscription".into());

    run_one(&e);

    assert_eq!(modes(&e), vec![(true, Some("subscription".to_string()))]);
}

#[test]
fn t_aij_auth_02_result_reported_mode_wins_over_the_configured_one() {
    // 구독 모드로 설정했지만 CLI 가 환경변수 API 키로 과금한 경우
    let e = AiEnv::new();
    *e.ext.auth_mode.lock().unwrap() = Some("subscription".into());
    let mut res = ai_env::ok_external();
    res.usage.auth_mode = Some("apiKey".into());
    e.ext.script.lock().unwrap().push_back(Ok(res));

    run_one(&e);

    assert_eq!(modes(&e), vec![(true, Some("apiKey".to_string()))]);
}

#[test]
fn t_aij_auth_03_failed_request_usage_records_the_configured_mode() {
    let e = AiEnv::new();
    *e.ext.auth_mode.lock().unwrap() = Some("subscription".into());
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("offline", "x")));

    run_one(&e);

    assert_eq!(modes(&e), vec![(false, Some("subscription".to_string()))]);
}

#[test]
fn t_aij_auth_04_analyzers_that_do_not_report_a_mode_store_null() {
    let e = AiEnv::new();

    run_one(&e);

    assert_eq!(modes(&e), vec![(true, None)]);
}

#[test]
fn t_aij_auth_05_usage_rows_split_by_mode_and_legacy_null_counts_as_api_key() {
    let e = AiEnv::new();
    let t0 = common::env::T0;
    let mk = move |mode: Option<&str>, at: i64| UsageEntry {
        provider: "claude".into(),
        model: "haiku".into(),
        kind: "describe".into(),
        asset_id: None,
        input_tokens: 10,
        output_tokens: 1,
        request_bytes: 100,
        images: 1,
        ok: true,
        error_code: None,
        at,
        auth_mode: mode.map(str::to_string),
    };
    e.w(move |c, _| {
        for u in [
            mk(None, t0),
            mk(Some("apiKey"), t0 + 1),
            mk(Some("subscription"), t0 + 2),
            mk(Some("subscription"), t0 + 3),
        ] {
            record_usage(c, &u).unwrap();
        }
    });

    let rows = e
        .env
        .read(|c| list_usage(c, "2023-11-14", "2023-11-16", 540).unwrap());

    let got: Vec<(Option<&str>, i64)> = rows
        .iter()
        .map(|r| (r.auth_mode.as_deref(), r.requests))
        .collect();
    assert_eq!(got, vec![(Some("apiKey"), 2), (Some("subscription"), 2)]);
    let json = serde_json::to_value(&rows[1]).unwrap();
    assert_eq!(json["authMode"], "subscription");
}
