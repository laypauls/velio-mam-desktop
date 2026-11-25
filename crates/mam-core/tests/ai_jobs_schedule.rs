//! T-aij-11~14, 26~28 - 등록 함수(신규 자동·백그라운드 스윕·일괄·재시도·동의 철회), 시간대, 설정, 쿼리 계획.
//! 분류: Business rule / Time / Boundary / Happy / Concurrency(배압).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::*;

fn jobs(e: &AiEnv) -> i64 {
    e.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai'")
}

fn snap(e: &AiEnv) -> AiRuntimeSettings {
    e.settings.snapshot()
}

#[test]
fn t_aij_11_new_asset_is_queued_only_with_auto_analyze_and_consent() {
    let e = AiEnv::new();
    let img = e.add_asset("image");
    let aud = e.add_asset("audio");
    let cases = [(false, true), (true, false)];
    for (auto, consent) in cases {
        e.set(|s| {
            s.auto_analyze_new = auto;
            s.consent_given = consent;
        });
        let r = e.sched.enqueue_new_asset(img, &snap(&e)).unwrap();
        assert_eq!(
            (r.queued, r.skipped),
            (0, 1),
            "auto={auto} consent={consent}"
        );
    }
    assert_eq!(
        (jobs(&e), e.env.count("SELECT COUNT(*) FROM ai_task_state")),
        (0, 0)
    );

    e.set(|s| {
        s.auto_analyze_new = true;
        s.consent_given = true;
    });
    let r = e.sched.enqueue_new_asset(img, &snap(&e)).unwrap();
    let ra = e.sched.enqueue_new_asset(aud, &snap(&e)).unwrap();
    let again = e.sched.enqueue_new_asset(img, &snap(&e)).unwrap();

    assert_eq!((r.queued, ra.queued), (1, 1));
    assert_eq!(jobs(&e), 2);
    assert_eq!(e.st(img, "describe"), "queued");
    assert_eq!(e.st(aud, "stt"), "queued");
    assert_eq!(
        e.st(aud, "describe"),
        "none",
        "음성은 외부 분석 대상이 아니다"
    );
    assert_eq!(again.queued, 0, "이미 대기 중이면 중복 등록 없음");
    assert_eq!(jobs(&e), 2);
}

#[test]
fn t_aij_11b_explicit_external_kinds_without_consent_are_skipped_not_queued() {
    let e = AiEnv::new();
    let img = e.add_asset("image");
    e.set(|s| s.consent_given = false);

    let r = e
        .sched
        .analyze_assets(&[img, 9_999], Some(&[StepKind::Describe]), false, &snap(&e))
        .unwrap();

    assert_eq!((r.queued, r.skipped), (0, 2), "동의 없음 + 없는 자산");
    assert_eq!(jobs(&e), 0);
    // 로컬 단계는 동의 없이도 등록된다
    let r = e
        .sched
        .analyze_assets(
            &[img],
            Some(&[StepKind::Describe, StepKind::Embedding]),
            false,
            &snap(&e),
        )
        .unwrap();
    assert_eq!(r.queued, 1);
    assert_eq!(
        (e.st(img, "embedding"), e.st(img, "describe")),
        ("queued".into(), "none".into())
    );
}

#[test]
fn t_aij_13_analyze_all_and_retry_failed_report_counts() {
    let e = AiEnv::new();
    let ids: Vec<i64> = (0..3).map(|_| e.add_asset("image")).collect();
    let inactive = e.add_asset("image");
    e.env
        .pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute(
                "UPDATE media_assets SET status = 'deleted' WHERE asset_id = ?1",
                [inactive],
            )?;
            Ok(())
        })
        .unwrap();
    e.queue_all(ids[0], None, false);

    // 미분석만: 이미 대기 중인 첫 자산은 제외
    let only = e.sched.analyze_all(true, &snap(&e)).unwrap();
    assert_eq!(only.queued, 2);
    // 전체 재분석(force): 활성 3개 전부
    let all = e.sched.analyze_all(false, &snap(&e)).unwrap();
    assert_eq!(all.queued, 3);
    assert_eq!(jobs(&e), 3, "대기 작업은 자산당 1개");

    // 실패 -> retry_failed(None) / 실패 없는 자산 지정은 skipped
    e.ext
        .script
        .lock()
        .unwrap()
        .extend((0..2).map(|_| Err(StepFailure::new("auth_failed", "키 확인"))));
    for _ in 0..3 {
        e.run_next(&|| false).unwrap().unwrap();
    }
    let failed_assets = e
        .env
        .count("SELECT COUNT(DISTINCT asset_id) FROM ai_task_state WHERE state = 'failed'");
    assert_eq!(failed_assets, 2);
    let none_failed = e.sched.retry_failed(Some(&[ids[2]])).unwrap();
    assert_eq!((none_failed.queued, none_failed.skipped), (0, 1));
    let r = e.sched.retry_failed(None).unwrap();
    assert_eq!((r.queued, r.skipped), (2, 0));
    assert_eq!(jobs(&e), 2);
}

#[test]
fn t_aij_14_consent_revocation_skips_queued_external_but_keeps_results_and_running() {
    let e = AiEnv::new();
    let done = e.add_asset("image");
    e.queue_all(done, None, false);
    e.run_next(&|| false).unwrap().unwrap(); // 기존 결과 보유
    let a = e.add_asset("video");
    let b = e.add_asset("image");
    e.queue_all(a, None, false);
    e.queue_all(b, None, false);
    e.env
        .pool
        .write(mam_db::Priority::Low, move |c| {
            c.execute(
                "UPDATE ai_task_state SET state = 'running' WHERE asset_id = ?1 AND kind = 'ocr'",
                [b],
            )?;
            Ok(())
        })
        .unwrap();

    let n = e.sched.cancel_external_pending().unwrap();

    assert_eq!(n, 4 + 3, "video 4 + image 3 (실행 중인 ocr 제외)");
    assert_eq!(
        e.step(a, "describe"),
        Some(("skipped".into(), Some("consent_required".into())))
    );
    assert_eq!(
        e.st(b, "ocr"),
        "running",
        "이미 실행 중인 요청은 끝까지 간다"
    );
    assert_eq!(
        (e.st(a, "stt"), e.st(a, "embedding")),
        ("queued".into(), "queued".into())
    );
    assert_eq!(e.st(done, "describe"), "done");
    assert_eq!(
        e.env.count("SELECT COUNT(*) FROM ai_analyses"),
        1,
        "기존 결과 보존"
    );
    // 취소 이후 실행: 외부 요청 없음, 로컬 단계는 수행
    e.set(|s| s.consent_given = false);
    let before = e.ext.calls.lock().unwrap().len();
    e.run_next(&|| false).unwrap().unwrap();
    assert_eq!(e.ext.calls.lock().unwrap().len(), before);
    assert_eq!(e.st(a, "stt"), "done");
}
