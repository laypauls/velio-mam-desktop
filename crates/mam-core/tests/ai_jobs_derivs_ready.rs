//! T-aij-40~45 - 필요한 파생물(썸네일·프레임 스트립)이 아직 만들어지는 중이면 AI 작업은 시도·경고 없이 기다린다.
//! 분류: Business rule(시도 비소모) / Time(not_before) / Concurrency(스윕·자동·수동 경로) / External(실패 파생물 대체).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::{AiSettingsSource, StepKind};
use mam_core::jobs::JobOutcome;

/// 대기 중 실행이 시도·이력·단계 오류를 하나도 남기지 않았는가 (= WARN 으로 이어지는 재시도 실패가 없다).
fn assert_no_attempt_spent(e: &AiEnv, id: i64, kind: &str) {
    assert_eq!(
        e.env
            .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'"),
        0,
        "작업 시도 횟수"
    );
    assert_eq!(
        e.step(id, kind),
        Some(("queued".into(), None)),
        "단계는 오류 코드 없이 queued"
    );
    assert_eq!(
        e.env.count("SELECT COUNT(*) FROM analysis_runs"),
        0,
        "실패 이력 없음"
    );
    assert!(e.ext.calls.lock().unwrap().is_empty(), "전송 없음");
}

fn sweep_on(e: &AiEnv) {
    e.set(|s| s.background_analyze = true);
}

#[test]
fn t_aij_40_video_with_pending_strip_waits_without_spending_attempts_then_runs() {
    for strip in [None, Some("pending")] {
        // Arrange: 스트립이 아직 없는 신규 영상 (썸네일은 있다), 자동 분석 경로로 등록
        let e = AiEnv::new();
        let id = e.add_asset("video");
        e.set_strip(id, strip, false);
        e.set(|s| {
            s.auto_analyze_new = true;
            s.stt_enabled = false;
        });
        let st = e.settings.snapshot();
        assert_eq!(e.sched.enqueue_new_asset(id, &st).unwrap().queued, 1);

        // Act: 여러 번 시도해도 기다리기만 한다
        for _ in 0..4 {
            let out = e.run_next(&|| false);
            // Assert
            assert_eq!(out, Some(Ok(JobOutcome::Yielded)), "{strip:?}");
            assert_no_attempt_spent(&e, id, "describe");
            assert!(e.emb.calls.lock().unwrap().is_empty());
            // not_before 가 미뤄졌으므로 시간이 흐르기 전엔 다시 꺼내지지 않는다 (바쁜 루프 없음)
            assert!(e.run_next(&|| false).is_none(), "{strip:?}");
            e.env.clock.advance(31_000);
        }

        // 스트립이 완료되면 같은 작업이 그대로 끝까지 실행된다
        e.set_strip(id, Some("done"), true);
        assert_eq!(
            e.run_next(&|| false),
            Some(Ok(JobOutcome::Done)),
            "{strip:?}"
        );
        assert_eq!(e.st(id, "describe"), "done");
        assert_eq!(e.st(id, "embedding"), "done");
        assert_eq!(e.ext.calls.lock().unwrap()[0].images.len(), 5);
    }
}

#[test]
fn t_aij_41_failed_or_unsupported_strip_falls_back_to_thumbnail_immediately() {
    for state in ["failed", "unsupported"] {
        // Arrange
        let e = AiEnv::new();
        let id = e.add_asset("video");
        e.set_strip(id, Some(state), false);
        e.queue_all(id, None, false);

        // Act
        let out = e.run_next(&|| false);

        // Assert: 기다리지 않고 썸네일 1장으로 바로 완료
        assert_eq!(out, Some(Ok(JobOutcome::Done)), "{state}");
        assert_eq!(e.ext.calls.lock().unwrap()[0].images.len(), 1, "{state}");
        assert_eq!(e.st(id, "describe"), "done");
    }
}

#[test]
fn t_aij_42_video_without_strip_whose_thumbnail_failed_does_not_wait_forever() {
    // Arrange: 썸네일 작업이 끝났는데(failed) 스트립 행은 영영 없다 -> 더 기다려도 생기지 않는다
    for thumb in ["failed", "unsupported"] {
        let e = AiEnv::new();
        let id = e.add_asset("video");
        e.set_strip(id, None, false);
        e.remove_thumb(id);
        e.set_thumb_state(id, Some(thumb));
        e.queue_all(id, Some(&[StepKind::Describe]), false);

        // Act
        let err = e.run_next(&|| false).unwrap().unwrap_err();

        // Assert: 기존 동작(thumbnail_missing 재시도)으로 이어진다
        assert!(err.retryable, "{thumb}");
        assert_eq!(
            e.step(id, "describe"),
            Some(("queued".into(), Some("thumbnail_missing".into())))
        );
    }
}

#[test]
fn t_aij_43_image_with_pending_thumbnail_waits_then_runs_via_the_sweep() {
    // Arrange: 썸네일 파일도 파생물 행도 아직 없는 신규 이미지, 백그라운드 스윕으로 등록
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.remove_thumb(id);
    e.set(|s| s.stt_enabled = false);
    sweep_on(&e);
    let st = e.settings.snapshot();
    assert_eq!(e.sched.background_sweep(&st, 0).unwrap().queued, 1);

    // Act + Assert: 기다림 (시도 비소모, 단계 오류 없음)
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));
    assert_eq!(
        e.env
            .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'"),
        0
    );
    assert_eq!(e.step(id, "describe"), Some(("queued".into(), None)));
    assert!(e.ext.calls.lock().unwrap().is_empty());
    // 이미 등록된 자산이라 스윕이 다시 고르지 않는다 (작업 한 건이 계속 대기)
    assert_eq!(e.sched.background_sweep(&st, 0).unwrap().queued, 0);

    // 썸네일이 만들어지면 실행된다
    e.put_thumb(id);
    e.set_thumb_state(id, Some("done"));
    e.env.clock.advance(31_000);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aij_44_audio_needs_no_derivatives() {
    // Arrange: 음성은 썸네일·스트립이 없다 (STT 는 원본을 읽는다)
    let e = AiEnv::new();
    let id = e.add_asset("audio");
    e.queue_all(id, None, false);

    // Act
    let out = e.run_next(&|| false);

    // Assert
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "stt"), "done");
    assert_eq!(e.st(id, "embedding"), "done");
    assert_eq!(e.order(), vec!["stt", "embedding"]);
}

#[test]
fn t_aij_45_manual_analyze_with_pending_strip_is_kept_and_eventually_runs_after_local_steps() {
    // Arrange: 수동 분석 요청 - STT 가 켜진 영상, 스트립 대기 중
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.set_strip(id, Some("pending"), false);
    e.set(|s| s.stt_enabled = true);
    let r = e.queue_all(id, None, false);
    assert_eq!(r.queued, 1);

    // Act 1: 로컬 STT 는 파생물과 무관하게 먼저 끝나고, 시각 단계만 기다린다
    let out = e.run_next(&|| false);

    // Assert 1
    assert_eq!(out, Some(Ok(JobOutcome::Yielded)));
    assert_eq!(e.st(id, "stt"), "done");
    assert_eq!(e.st(id, "describe"), "queued");
    assert_eq!(e.st(id, "embedding"), "queued");
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'"),
        1,
        "요청은 버려지지 않고 대기열에 남는다"
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0"),
        0
    );

    // Act 2: 스트립 완료 후 이어서 실행 - STT 는 다시 하지 않는다
    e.set_strip(id, Some("done"), true);
    e.env.clock.advance(31_000);
    let out = e.run_next(&|| false);

    // Assert 2
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(e.st(id, "embedding"), "done");
    assert_eq!(e.stt.calls.lock().unwrap().len(), 1, "STT 1회");
}

#[test]
fn t_aij_46_local_retry_failure_takes_precedence_over_waiting() {
    // Arrange: STT 가 일시 오류 + 스트립 대기 -> 실제 실패는 시도를 소모하는 재시도로 보고된다
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.set_strip(id, Some("pending"), false);
    e.stt
        .script
        .lock()
        .unwrap()
        .push_back(Err(mam_core::ai_jobs::StepFailure::new("timeout", "t")));
    e.queue_all(id, None, false);

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert!(out.unwrap_err().retryable);
    assert_eq!(e.step(id, "stt").unwrap().0, "queued");
    assert_eq!(
        e.env.count("SELECT attempts FROM jobs WHERE kind = 'ai'"),
        1,
        "실제 실패는 시도를 센다"
    );
}
