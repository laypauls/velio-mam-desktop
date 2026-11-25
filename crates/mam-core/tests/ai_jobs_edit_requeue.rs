//! T-aij-30~34 - 사용자 편집·전사 갱신 뒤 낡은 의미 벡터 재계산: 의미 전용 재대기, 외부 분석·시각 벡터 불변, 설정/재시도 경로.
//! 분류: Business rule / Concurrency(중복 등록) / External(일시 오류) / Boundary(변경 없음·행 없음).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::{EmbedMode, StepFailure, StepKind};
use mam_core::ai_store::{AnalysisEdit, load_analysis};
use mam_core::jobs::{JobOutcome, backoff_ms};

fn edit_text(text: &str) -> AnalysisEdit {
    AnalysisEdit {
        description: Some(text.into()),
        keywords: None,
    }
}

/// 영상 1개를 모든 단계 완료까지 돌리고 호출 기록을 비운다 (실제 벡터 저장소 연결).
fn analyzed_video() -> (AiEnv, i64) {
    let e = AiEnv::new();
    e.enable_vectors();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    e.ext.calls.lock().unwrap().clear();
    e.stt.calls.lock().unwrap().clear();
    e.emb.calls.lock().unwrap().clear();
    e.log.lock().unwrap().clear();
    (e, id)
}

fn pending_ai_jobs(e: &AiEnv) -> i64 {
    e.env
        .count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'")
}

#[test]
fn t_aij_30_user_edit_requeues_semantic_only_without_external_call_or_visual_change() {
    // Arrange
    let (e, id) = analyzed_video();
    let visual = e.vector_ids(id, "visual");
    let semantic = e.vector_ids(id, "semantic");
    assert_eq!((visual.len(), semantic.len()), (5, 1));

    // Act: 편집 -> 재대기 확인 -> 작업 실행
    let view = e
        .sched
        .edit_analysis(id, &edit_text("handmade text"))
        .unwrap();
    let queued = (e.step(id, "embedding"), pending_ai_jobs(&e));
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(
        queued,
        (Some(("queued".into(), Some("semantic_only".into()))), 1)
    );
    assert!(view.edited);
    assert_eq!(out, Ok(JobOutcome::Done));
    assert!(
        e.ext.calls.lock().unwrap().is_empty(),
        "사용자 편집은 외부 재분석을 일으키지 않는다"
    );
    assert!(e.stt.calls.lock().unwrap().is_empty());
    let calls = e.emb.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].mode, EmbedMode::SemanticOnly);
    assert!(calls[0].images.is_empty());
    assert!(calls[0].text_parts.join("|").contains("handmade text"));
    drop(calls);
    assert_eq!(e.vector_ids(id, "visual"), visual, "시각 벡터는 그대로");
    let new_semantic = e.vector_ids(id, "semantic");
    assert_eq!(new_semantic.len(), 1);
    assert_ne!(new_semantic, semantic, "의미 벡터는 새로 만들어졌다");
    assert_eq!(e.step(id, "embedding"), Some(("done".into(), None)));
    for k in ["describe", "keywords", "ocr", "summary", "stt"] {
        assert_eq!(e.st(id, k), "done", "{k}");
    }
    let saved = e.env.read(|c| load_analysis(c, id).unwrap().unwrap());
    assert!(saved.edited && saved.description == "handmade text");
}

#[test]
fn t_aij_30b_edit_does_not_requeue_when_nothing_to_refresh() {
    // 변경 없음: 같은 설명으로 저장
    let (e, id) = analyzed_video();
    e.sched.edit_analysis(id, &edit_text("a red car")).unwrap();
    assert_eq!(
        e.st(id, "embedding"),
        "done",
        "텍스트가 같으면 재계산 안 함"
    );
    assert_eq!(pending_ai_jobs(&e), 0);

    // 분석을 한 적 없는 자산: 단계 행을 만들지 않는다 (스윕이 시각 벡터까지 만들도록)
    let fresh = e.add_asset("image");
    e.sched.edit_analysis(fresh, &edit_text("note")).unwrap();
    assert_eq!(
        e.env.count(&format!(
            "SELECT COUNT(*) FROM ai_task_state WHERE asset_id = {fresh}"
        )),
        0
    );
    assert_eq!(pending_ai_jobs(&e), 0);

    // 이미 전체 임베딩이 대기 중이면 의미 전용으로 낮추지 않는다
    let pending = e.add_asset("image");
    e.queue_all(pending, Some(&[StepKind::Embedding]), false);
    e.sched.edit_analysis(pending, &edit_text("note")).unwrap();
    assert_eq!(e.step(pending, "embedding"), Some(("queued".into(), None)));
    e.run_next(&|| false).unwrap().unwrap();
    let calls = e.emb.calls.lock().unwrap();
    assert_eq!(calls[0].mode, EmbedMode::Full);
    assert_eq!(calls[0].images.len(), 1);
    drop(calls);

    // 실패한 임베딩은 건드리지 않는다 (수동 재시도가 전체를 다시 한다)
    let bad = e.add_asset("image");
    e.queue_all(bad, Some(&[StepKind::Embedding]), false);
    e.emb
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("invalid_input", "bad")));
    e.run_next(&|| false).unwrap().unwrap();
    assert_eq!(e.st(bad, "embedding"), "failed");
    e.sched.edit_analysis(bad, &edit_text("note")).unwrap();
    assert_eq!(e.st(bad, "embedding"), "failed");
}

#[test]
fn t_aij_31_semantic_search_disabled_skips_the_requeued_step() {
    let (e, id) = analyzed_video();
    e.sched.edit_analysis(id, &edit_text("new words")).unwrap();
    e.set(|s| s.semantic_enabled = false);

    let out = e.run_next(&|| false).unwrap();

    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(
        e.step(id, "embedding"),
        Some(("skipped".into(), Some("disabled".into())))
    );
    assert!(e.emb.calls.lock().unwrap().is_empty());
    assert!(e.ext.calls.lock().unwrap().is_empty());
}

#[test]
fn t_aij_32_transient_failure_retries_as_semantic_only() {
    // Arrange
    let (e, id) = analyzed_video();
    e.sched.edit_analysis(id, &edit_text("new words")).unwrap();
    e.emb
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("io_transient", "disk busy")));

    // Act
    let first = e.run_next(&|| false).unwrap();
    let after_fail = e.step(id, "embedding");
    e.env.clock.advance(backoff_ms(1));
    let second = e.run_next(&|| false).unwrap();

    // Assert
    assert!(first.unwrap_err().retryable);
    assert_eq!(
        after_fail,
        Some(("queued".into(), Some("semantic_only".into())))
    );
    assert_eq!(second, Ok(JobOutcome::Done));
    let calls = e.emb.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(
        calls
            .iter()
            .all(|c| c.mode == EmbedMode::SemanticOnly && c.images.is_empty())
    );
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert_eq!(e.st(id, "embedding"), "done");
}

#[test]
fn t_aij_32b_transcript_resave_refreshes_semantic_vector_only() {
    // Arrange: 전체 완료 후 STT 만 다시 실행 (force)
    let (e, id) = analyzed_video();
    let visual = e.vector_ids(id, "visual");
    let semantic = e.vector_ids(id, "semantic");
    e.queue_all(id, Some(&[StepKind::Stt]), true);

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: stt -> embedding(의미 전용), 외부 호출 없음
    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(e.order(), vec!["stt", "embedding"]);
    let calls = e.emb.calls.lock().unwrap();
    assert_eq!(calls[0].mode, EmbedMode::SemanticOnly);
    assert!(calls[0].text_parts.join("|").contains("hello world"));
    drop(calls);
    assert_eq!(e.vector_ids(id, "visual"), visual);
    assert_ne!(e.vector_ids(id, "semantic"), semantic);
    assert_eq!(e.st(id, "embedding"), "done");
}

#[test]
fn t_aij_33_repeated_edits_register_one_job_and_need_no_thumbnail() {
    // Arrange: 썸네일·프레임이 없어도 의미 전용 재계산은 가능하다
    let (e, id) = analyzed_video();
    e.remove_thumb(id);
    e.set_strip(id, None, false);

    // Act
    e.sched.edit_analysis(id, &edit_text("first")).unwrap();
    e.sched.edit_analysis(id, &edit_text("second")).unwrap();
    let jobs = pending_ai_jobs(&e);
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(jobs, 1, "대기 작업은 중복 등록하지 않는다");
    assert_eq!(out, Ok(JobOutcome::Done));
    let calls = e.emb.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].text_parts.join("|").contains("second"));
    drop(calls);
    assert!(e.run_next(&|| false).is_none());
}
