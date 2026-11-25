//! T-aij-04, 05, 15, 16 - 없는 자산·사용량 집계(일 경계)·파일별 이력·CASCADE (실제 SQLite + 제안 DDL).
//! 분류: Time / Boundary / Validation / Business rule.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_store::{
    AiStoreError, AnalysisEdit, AnalysisFields, NewAnalysis, RunRecord, UsageEntry, list_runs,
    list_usage, record_run, record_usage, save_analysis, update_analysis,
};

fn new_analysis(fields: AnalysisFields) -> NewAnalysis {
    NewAnalysis {
        fields,
        language: "ko".into(),
        provider: "openai".into(),
        model: "gpt-x".into(),
    }
}

fn full() -> AnalysisFields {
    AnalysisFields {
        description: Some("a red car".into()),
        ..AnalysisFields::default()
    }
}

#[test]
fn t_aij_04_usage_is_recorded_and_aggregated_per_local_day_provider_model() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    let t0 = common::env::T0;
    let u = move |at: i64, ok: bool, model: &str| UsageEntry {
        provider: "openai".into(),
        model: model.into(),
        kind: "describe+keywords".into(),
        asset_id: Some(id),
        input_tokens: 100,
        output_tokens: 10,
        request_bytes: 1000,
        images: 1,
        ok,
        error_code: (!ok).then(|| "offline".to_string()),
        at,
        auth_mode: None,
    };
    // KST(+540분): t0 = 11-15 07:13, t0-8h = 11-14 23:13 (UTC 14:13)
    e.w(move |c, _| {
        for (at, ok, m) in [
            (t0, true, "a"),
            (t0 + 7_200_000, false, "a"),
            (t0 - 28_800_000, true, "a"),
            (t0, true, "b"),
        ] {
            record_usage(c, &u(at, ok, m)).unwrap();
        }
    });

    let rows = e
        .env
        .read(|c| list_usage(c, "2023-11-14", "2023-11-15", 540).unwrap());
    let key: Vec<(&str, &str, i64, i64, i64, i64)> = rows
        .iter()
        .map(|r| {
            (
                r.day.as_str(),
                r.model.as_str(),
                r.requests,
                r.failures,
                r.input_tokens,
                r.images,
            )
        })
        .collect();
    assert_eq!(
        key,
        vec![
            ("2023-11-14", "a", 1, 0, 100, 1),
            ("2023-11-15", "a", 2, 1, 200, 2),
            ("2023-11-15", "b", 1, 0, 100, 1),
        ]
    );
    // 경계: 범위 밖 날짜는 제외, 잘못된 형식·역순 범위는 검증 오류
    assert!(
        e.env
            .read(|c| list_usage(c, "2023-11-16", "2023-11-16", 540).unwrap())
            .is_empty()
    );
    assert!(matches!(
        e.env.read(|c| list_usage(c, "2023/11/14", "2023-11-15", 0)),
        Err(AiStoreError::Invalid(_))
    ));
    assert!(matches!(
        e.env.read(|c| list_usage(c, "2023-11-15", "2023-11-14", 0)),
        Err(AiStoreError::Invalid(_))
    ));
}

#[test]
fn t_aij_15_runs_list_newest_first_and_message_is_bounded() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    let rec = move |ok: bool, msg: String| RunRecord {
        asset_id: id,
        kind: "describe".into(),
        provider: "openai".into(),
        ok,
        error_code: (!ok).then(|| "auth_failed".to_string()),
        message: msg,
        attempts: 2,
    };
    e.w(move |c, clk| {
        record_run(c, clk, &rec(false, "API 키를 확인하세요".into())).unwrap();
    });
    e.env.clock.advance(1000);
    e.w(move |c, clk| {
        record_run(c, clk, &rec(true, "x".repeat(5000))).unwrap();
    });

    let runs = e.env.read(|c| list_runs(c, id, 10).unwrap());

    assert_eq!(
        runs.iter().map(|r| r.state).collect::<Vec<_>>(),
        vec!["done", "failed"]
    );
    assert_eq!(runs[1].error_code.as_deref(), Some("auth_failed"));
    let longest: i64 = e
        .env
        .count("SELECT MAX(length(message)) FROM analysis_runs");
    assert_eq!(longest, 200, "메시지 길이 상한");
    assert_eq!(e.env.read(|c| list_runs(c, id, 1).unwrap()).len(), 1);
}

#[test]
fn t_aij_16_cascade_on_asset_delete_but_usage_survives() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())).unwrap());
    e.w(move |c, clk| {
        record_run(
            c,
            clk,
            &RunRecord {
                asset_id: id,
                kind: "ocr".into(),
                provider: "openai".into(),
                ok: true,
                error_code: None,
                message: String::new(),
                attempts: 1,
            },
        )
        .unwrap();
        record_usage(
            c,
            &UsageEntry {
                provider: "openai".into(),
                model: "m".into(),
                kind: "ocr".into(),
                asset_id: Some(id),
                input_tokens: 1,
                output_tokens: 1,
                request_bytes: 1,
                images: 1,
                ok: true,
                error_code: None,
                at: 1,
                auth_mode: None,
            },
        )
        .unwrap();
    });

    e.env
        .pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute("DELETE FROM file_locations WHERE asset_id = ?1", [id])?;
            c.execute("DELETE FROM media_assets WHERE asset_id = ?1", [id])?;
            Ok(())
        })
        .unwrap();

    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 0);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM analysis_runs"), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM ai_usage WHERE asset_id IS NULL"),
        1
    );
}
#[test]
fn t_aij_05_missing_or_inactive_asset_changes_nothing() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.env
        .pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute(
                "UPDATE media_assets SET status = 'superseded' WHERE asset_id = ?1",
                [id],
            )?;
            Ok(())
        })
        .unwrap();
    let ob0 = e.env.count("SELECT COUNT(*) FROM search_outbox");

    let inactive = e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())));
    let missing = e.w(move |c, clk| save_analysis(c, clk, 9_999, &new_analysis(full())));
    let edit_missing = e.w(move |c, clk| update_analysis(c, clk, 9_999, &AnalysisEdit::default()));

    assert!(matches!(inactive, Err(AiStoreError::NotFound(_))));
    assert!(matches!(missing, Err(AiStoreError::NotFound(9_999))));
    assert!(matches!(edit_missing, Err(AiStoreError::NotFound(_))));
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 0);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM search_outbox"), ob0);
}
