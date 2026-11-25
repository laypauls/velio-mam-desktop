//! T-aij-01~05, 15~17 - AI 분석 저장·편집·재분석 덮어쓰기·이력·사용량 (실제 SQLite + 마이그레이션 + 제안 DDL).
//! 분류: Happy / Business rule / Validation / External(없는 자산) / Time(일 경계) / Boundary.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_store::{
    AiStoreError, AnalysisEdit, AnalysisFields, NewAnalysis, load_analysis, save_analysis,
    update_analysis,
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
        keywords: Some(vec![" car ".into(), "red".into(), "car".into(), "".into()]),
        ocr_text: Some("STOP".into()),
        summary: Some("photo".into()),
    }
}

fn outbox(e: &AiEnv, id: i64) -> Vec<(i64, i64)> {
    e.env.read(|c| {
        let mut s = c
            .prepare("SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id")
            .unwrap();
        s.query_map([id], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

fn revision(e: &AiEnv, id: i64) -> i64 {
    e.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ))
}

#[test]
fn t_aij_01_save_writes_row_bumps_revision_and_one_low_priority_outbox() {
    // Arrange
    let e = AiEnv::new();
    let id = e.add_asset("image");
    let (r0, o0) = (revision(&e, id), outbox(&e, id).len());

    // Act
    let out = e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())).unwrap());

    // Assert: 키워드는 정리(공백·빈 값·중복 제거), revision+1, outbox 저우선(1) 1건
    assert_eq!(out.revision, r0 + 1);
    assert_eq!(revision(&e, id), r0 + 1);
    let ob = outbox(&e, id);
    assert_eq!(ob.len(), o0 + 1);
    assert_eq!(ob.last().copied(), Some((r0 + 1, 1)));
    let a = e.env.read(|c| load_analysis(c, id).unwrap().unwrap());
    assert_eq!(a.keywords, vec!["car", "red"]);
    assert_eq!(
        (a.description.as_str(), a.provider.as_str(), a.edited),
        ("a red car", "openai", false)
    );
}

#[test]
fn t_aij_02_user_edit_marks_edited_and_enqueues_high_priority_outbox() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())).unwrap());

    let view = e.w(move |c, clk| {
        let edit = AnalysisEdit {
            description: Some("my text".into()),
            keywords: None,
        };
        update_analysis(c, clk, id, &edit).unwrap()
    });

    assert!(view.edited);
    assert_eq!(
        (view.description.as_str(), view.keywords.len()),
        ("my text", 2)
    );
    assert_eq!(view.ocr_text, "STOP", "편집하지 않은 필드는 유지");
    assert_eq!(
        outbox(&e, id).last().map(|o| o.1),
        Some(0),
        "사용자 편집 = 고우선 0"
    );
}

#[test]
fn t_aij_02b_edit_validation_failures_change_nothing() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    let r0 = revision(&e, id);

    let too_long = e.w(move |c, clk| {
        let edit = AnalysisEdit {
            description: Some("가".repeat(20_001)),
            keywords: None,
        };
        update_analysis(c, clk, id, &edit)
    });
    let kw_long = e.w(move |c, clk| {
        let edit = AnalysisEdit {
            description: None,
            keywords: Some(vec!["x".repeat(101)]),
        };
        update_analysis(c, clk, id, &edit)
    });

    assert!(matches!(too_long, Err(AiStoreError::Invalid(_))));
    assert!(matches!(kw_long, Err(AiStoreError::Invalid(_))));
    assert_eq!(revision(&e, id), r0);
    assert!(e.env.read(|c| load_analysis(c, id).unwrap()).is_none());
}

#[test]
fn t_aij_03_reanalysis_overwrites_and_keeps_replaced_edit_in_history() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())).unwrap());
    e.w(move |c, clk| {
        let edit = AnalysisEdit {
            description: Some("my text".into()),
            keywords: Some(vec!["mine".into()]),
        };
        update_analysis(c, clk, id, &edit).unwrap()
    });

    // 재분석(전체) -> 덮어쓰기
    let again = AnalysisFields {
        description: Some("new desc".into()),
        keywords: Some(vec!["new".into()]),
        ocr_text: Some("".into()),
        summary: Some("s2".into()),
    };
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(again)).unwrap());

    let a = e.env.read(|c| load_analysis(c, id).unwrap().unwrap());
    assert_eq!(
        (a.description.as_str(), a.edited, a.keywords.clone()),
        ("new desc", false, vec!["new".to_string()])
    );
    let saved: (String, String) = e.env.read(|c| {
        c.query_row(
            "SELECT description, keywords FROM ai_analysis_replaced WHERE asset_id = ?1",
            [id],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    });
    assert_eq!(saved, ("my text".to_string(), "[\"mine\"]".to_string()));
}

#[test]
fn t_aij_03b_partial_reanalysis_keeps_user_edit_and_other_fields() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(full())).unwrap());
    e.w(move |c, clk| {
        let edit = AnalysisEdit {
            description: Some("my text".into()),
            keywords: None,
        };
        update_analysis(c, clk, id, &edit).unwrap()
    });

    // OCR 만 재분석 -> 설명·키워드(편집 영역)는 덮어쓰지 않는다
    let only_ocr = AnalysisFields {
        ocr_text: Some("NEW".into()),
        ..AnalysisFields::default()
    };
    e.w(move |c, clk| save_analysis(c, clk, id, &new_analysis(only_ocr)).unwrap());

    let a = e.env.read(|c| load_analysis(c, id).unwrap().unwrap());
    assert_eq!(
        (a.description.as_str(), a.ocr_text.as_str(), a.edited),
        ("my text", "NEW", true)
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analysis_replaced"), 0);
}
