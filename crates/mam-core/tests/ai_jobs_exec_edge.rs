//! T-aij-21~25 - AI 작업 실행기 경계: 음성(원본 비전송·보호), 모델 없음/비활성, STT 제한 시간, 비활성 자산, 중단.
//! 분류: Business rule / Boundary / Time / External.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use std::time::Duration;

use ai_env::AiEnv;
use mam_core::ai_jobs::{StepFailure, StepKind, SttTimeout};
use mam_core::jobs::JobOutcome;

#[test]
fn t_aij_21_audio_runs_local_steps_only_and_never_touches_the_original() {
    let e = AiEnv::new();
    let id = e.add_asset("audio");
    let before = common::env::snapshot(&e.env.vol);

    let q = e.queue_all(id, None, false);
    let out = e.run_next(&|| false).unwrap();

    assert_eq!((q.queued, out), (1, Ok(JobOutcome::Done)));
    assert_eq!(
        e.order(),
        vec!["stt", "embedding"],
        "음성 원본은 외부로 보내지 않는다"
    );
    assert!(e.emb.calls.lock().unwrap()[0].images.is_empty());
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_task_state WHERE kind IN ('describe','keywords','ocr','summary')"), 0);
    assert_eq!(common::env::snapshot(&e.env.vol), before, "원본 보호");
}

#[test]
fn t_aij_22_model_missing_skips_instead_of_failing_and_disabled_stt_is_skipped() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.set(|s| s.stt_enabled = false);
    e.emb.script.lock().unwrap().push_back(Err(StepFailure::new(
        "model_missing",
        "모델이 설치되지 않았습니다",
    )));

    let out = e.run_next(&|| false).unwrap();

    assert_eq!(out, Ok(JobOutcome::Done));
    assert_eq!(
        e.step(id, "embedding"),
        Some(("skipped".into(), Some("model_missing".into())))
    );
    assert_eq!(
        e.step(id, "stt"),
        Some(("skipped".into(), Some("disabled".into())))
    );
    assert!(e.stt.calls.lock().unwrap().is_empty());
}

#[test]
fn t_aij_23_stt_timeout_scales_with_duration_and_is_capped() {
    let t = SttTimeout {
        base_secs: 120,
        per_media_sec: 2.0,
        cap_secs: 3600,
    };
    assert_eq!(t.for_duration(Some(0)), Duration::from_secs(120));
    assert_eq!(t.for_duration(Some(60_000)), Duration::from_secs(240));
    assert_eq!(
        t.for_duration(Some(10 * 3600 * 1000)),
        Duration::from_secs(3600),
        "상한"
    );
    assert_eq!(
        t.for_duration(None),
        Duration::from_secs(3600),
        "길이를 모르면 상한"
    );
    assert_eq!(
        SttTimeout::default().for_duration(Some(1_000)),
        Duration::from_secs(122)
    );

    // 실행기가 자산 길이(60초)로 계산한 값을 STT 에 전달한다 (기본 120 + 2x60)
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, Some(&[StepKind::Stt]), false);
    e.run_next(&|| false).unwrap().unwrap();
    assert_eq!(
        e.stt.calls.lock().unwrap()[0].timeout,
        Duration::from_secs(240)
    );
}

#[test]
fn t_aij_24_asset_that_became_inactive_is_skipped_not_failed() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, None, false);
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

    let out = e.run_next(&|| false).unwrap();

    assert_eq!(out, Ok(JobOutcome::Done));
    assert!(e.order().is_empty());
    assert_eq!(
        e.step(id, "describe"),
        Some(("skipped".into(), Some("asset_inactive".into())))
    );
}

#[test]
fn t_aij_25_cancelled_stt_yields_without_counting_an_attempt() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.stt
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("cancelled", "중단됨")));

    let out = e.run_next(&|| false).unwrap();

    assert_eq!(out, Ok(JobOutcome::Yielded));
    assert_eq!(
        e.step(id, "stt"),
        Some(("queued".into(), Some("cancelled".into())))
    );
    assert_eq!(e.env.count("SELECT attempts FROM jobs"), 0);
    assert_eq!(e.ext.calls.lock().unwrap().len(), 0);
}
