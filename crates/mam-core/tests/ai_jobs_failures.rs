//! T-aij-07, 09, 18~20 - AI 작업 실패 경로: 오프라인 재시도, 인증 실패(영구), 429 Retry-After, 시도 한도, 원본 오프라인.
//! 분류: External / Business rule / Time / Concurrency(재실행 시 완료 단계 보존).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use std::time::Duration;

use ai_env::AiEnv;
use mam_core::ai_jobs::{StepFailure, StepKind};
use mam_core::jobs::{JobError, JobOutcome, MAX_ATTEMPTS, backoff_ms};

fn not_before(e: &AiEnv) -> i64 {
    e.env.count("SELECT not_before FROM jobs")
}

#[test]
fn t_aij_07_offline_external_is_retryable_while_local_results_are_kept() {
    // Arrange
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.ext.script.lock().unwrap().push_back(Err(StepFailure::new(
        "offline",
        "네트워크에 연결할 수 없어 외부 AI 분석만 중단됩니다",
    )));

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert: 재시도 가능 오류 + 로컬 단계 보존, 외부 단계는 queued(+코드), 백오프 후 재시도
    let err = out.unwrap_err();
    assert!(err.retryable && err.retry_after.is_none());
    assert!(
        !err.message.contains("네트워크"),
        "작업 오류에는 코드만: {}",
        err.message
    );
    assert_eq!(e.st(id, "stt"), "done");
    assert_eq!(e.st(id, "embedding"), "done");
    assert_eq!(
        e.step(id, "describe"),
        Some(("queued".into(), Some("offline".into())))
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM transcripts"), 1);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_analyses"), 0);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM ai_usage WHERE ok = 0 AND error_code = 'offline'"),
        1
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0 AND error_code = 'offline'"),
        4
    );
    assert_eq!(not_before(&e), common::env::T0 + backoff_ms(1));
    assert!(
        e.run_next(&|| false).is_none(),
        "백오프 전에는 다시 시작하지 않는다"
    );

    // 백오프 후 재시도: 외부 단계만 실행(stt 재실행 없음), 새 AI 텍스트로 임베딩 갱신
    e.env.clock.advance(backoff_ms(1));
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    assert_eq!(e.stt.calls.lock().unwrap().len(), 1);
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(e.emb.calls.lock().unwrap().len(), 2);
    assert!(
        e.emb.calls.lock().unwrap()[1]
            .text_parts
            .join("|")
            .contains("a red car")
    );
}

#[test]
fn t_aij_09_auth_failure_is_permanent_and_recorded_without_retry() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, Some(&[StepKind::Describe, StepKind::Ocr]), false);
    e.ext
        .script
        .lock()
        .unwrap()
        .push_back(Err(StepFailure::new("auth_failed", "API 키를 확인하세요")));

    let out = e.run_next(&|| false).unwrap();

    assert_eq!(out, Ok(JobOutcome::Done), "영구 실패는 작업 재시도 없음");
    assert_eq!(
        e.step(id, "describe"),
        Some(("failed".into(), Some("auth_failed".into())))
    );
    assert_eq!(
        e.step(id, "ocr"),
        Some(("failed".into(), Some("auth_failed".into())))
    );
    assert_eq!(e.env.count("SELECT COUNT(*) FROM jobs"), 0);
    let msg: String = e.env.read(|c| {
        c.query_row("SELECT message FROM analysis_runs LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap()
    });
    assert_eq!(msg, "API 키를 확인하세요", "이력에는 정제된 메시지만");
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0 AND provider = 'openai'"),
        2
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM ai_usage WHERE ok = 0 AND error_code = 'auth_failed'"),
        1
    );
    assert!(e.run_next(&|| false).is_none());
}

#[test]
fn t_aij_18_rate_limit_retry_after_becomes_job_not_before() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, Some(&[StepKind::Describe]), false);
    e.ext.script.lock().unwrap().push_back(Err(
        StepFailure::new("rate_limited", "요청 한도").with_retry_after(Some(600))
    ));

    let err = e.run_next(&|| false).unwrap().unwrap_err();

    assert_eq!(
        err,
        JobError::retryable_after(
            "ai steps will retry: rate_limited",
            Duration::from_secs(600)
        )
    );
    assert_eq!(
        not_before(&e),
        common::env::T0 + 600_000,
        "Retry-After 가 백오프(30초)보다 길다"
    );
}

#[test]
fn t_aij_18b_short_retry_after_does_not_shorten_backoff() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, Some(&[StepKind::Describe]), false);
    e.ext.script.lock().unwrap().push_back(Err(
        StepFailure::new("rate_limited", "x").with_retry_after(Some(1))
    ));

    e.run_next(&|| false).unwrap().unwrap_err();

    assert_eq!(not_before(&e), common::env::T0 + backoff_ms(1));
}

#[test]
fn t_aij_19_retries_are_exhausted_into_failed_steps_and_manual_retry_recovers() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, Some(&[StepKind::Describe]), false);
    for _ in 0..MAX_ATTEMPTS {
        e.ext
            .script
            .lock()
            .unwrap()
            .push_back(Err(StepFailure::new("server_error", "서버 오류")));
    }

    // Act: 시도 한도까지 반복
    for _ in 0..MAX_ATTEMPTS {
        e.env.clock.advance(3_600_000);
        assert!(e.run_next(&|| false).unwrap().is_err());
    }

    // Assert: 단계 failed, 작업 failed, 실패 이력이 시도마다 남음
    assert_eq!(
        e.step(id, "describe"),
        Some(("failed".into(), Some("server_error".into())))
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        1
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM analysis_runs WHERE ok = 0"),
        MAX_ATTEMPTS
    );
    assert_eq!(
        e.env.count("SELECT MAX(attempts) FROM analysis_runs"),
        MAX_ATTEMPTS
    );

    // 수동 재시도: 실패 단계를 다시 대기시키고 실패한 작업 행은 새 작업으로 대체
    let r = e.sched.retry_failed(Some(&[id])).unwrap();
    assert_eq!(r.queued, 1);
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        0
    );
    assert_eq!(e.run_next(&|| false).unwrap(), Ok(JobOutcome::Done));
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aij_20_unavailable_original_retries_stt_but_external_still_runs() {
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.queue_all(id, None, false);
    e.env
        .pool
        .write(mam_db::Priority::Low, |c| {
            c.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();

    let out = e.run_next(&|| false).unwrap();

    assert!(out.unwrap_err().retryable);
    assert_eq!(
        e.step(id, "stt"),
        Some(("queued".into(), Some("source_unavailable".into())))
    );
    assert!(
        e.stt.calls.lock().unwrap().is_empty(),
        "오프라인 원본은 읽지 않는다"
    );
    assert_eq!(
        e.st(id, "describe"),
        "done",
        "썸네일 기반 외부 분석은 오프라인 볼륨과 무관"
    );
    assert_eq!(
        e.env
            .count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        1,
        "삭제로 취급하지 않는다"
    );
}

#[test]
fn t_aij_20b_missing_thumbnail_is_retryable_and_sends_nothing() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.remove_thumb(id);
    e.set_thumb_state(id, Some("failed")); // 썸네일 작업이 끝났는데 파일이 없다 (아직 만드는 중이면 T-aij-43)
    e.queue_all(id, Some(&[StepKind::Describe]), false);

    let err = e.run_next(&|| false).unwrap().unwrap_err();

    assert!(err.retryable);
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert_eq!(
        e.step(id, "describe"),
        Some(("queued".into(), Some("thumbnail_missing".into())))
    );
    assert_eq!(
        e.env.count("SELECT COUNT(*) FROM ai_usage"),
        0,
        "요청이 나가지 않았으면 사용량도 없다"
    );
}
