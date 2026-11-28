//! T-subtitle-01~ - 새 파일 자동 자막 생성(동의 불필요·종류 합성)·종류별 정리·실행 게이트·자막 일괄 생성.
//! 분류: Happy / Boundary(500건 페이지) / Business rule(옵션 조합·종류별 정리) / Concurrency(멱등·두 번 실행) / External(꺼짐 거부).
//! `stt` = 자막 (저장·교환 값은 `stt` 유지).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::*;
use mam_core::jobs::{JobKind, JobOutcome};

fn snap(e: &AiEnv) -> AiRuntimeSettings {
    e.settings.snapshot()
}

fn exec(e: &AiEnv, sql: &str) {
    let sql = sql.to_string();
    e.w(move |c, _| c.execute(&sql, []).unwrap());
}

fn jobs(e: &AiEnv) -> i64 {
    e.env
        .count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'")
}

fn steps(e: &AiEnv) -> i64 {
    e.env.count("SELECT COUNT(*) FROM ai_task_state")
}

fn origin_of(e: &AiEnv, asset: i64, kind: &str) -> Option<String> {
    e.env.read(|c| {
        c.query_row(
            "SELECT origin FROM ai_task_state WHERE asset_id = ?1 AND kind = ?2",
            (asset, kind),
            |r| r.get(0),
        )
        .unwrap()
    })
}

/// 자막 상태를 직접 만든다 (행 교체).
fn put_step(e: &AiEnv, id: i64, kind: &str, state: &str, origin: &str) {
    exec(
        e,
        &format!(
            "INSERT OR REPLACE INTO ai_task_state (asset_id, kind, state, attempts, updated_at, origin)
             VALUES ({id}, '{kind}', '{state}', 0, 1, '{origin}')"
        ),
    );
}

/// 동의 없음 + 자동 자막 켬 (자동 분석 꺼짐).
fn subtitle_only(e: &AiEnv) {
    e.set(|s| {
        s.consent_given = false;
        s.auto_analyze_new = false;
        s.auto_subtitle_new = true;
    });
}

/// 동의 + 자동 분석 켬 (자동 자막 꺼짐). 의미 검색은 끈다.
fn analyze_only(e: &AiEnv) {
    e.set(|s| {
        s.consent_given = true;
        s.auto_analyze_new = true;
        s.auto_subtitle_new = false;
        s.semantic_enabled = false;
    });
}

// ── B7. 새 파일 등록 ────────────────────────────────────────────────────

#[test]
fn t_subtitle_01_auto_subtitle_without_consent_queues_only_stt() {
    // Arrange
    let e = AiEnv::new();
    subtitle_only(&e);
    let video = e.add_asset("video");

    // Act
    let r = e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();

    // Assert
    assert_eq!((r.queued, r.skipped), (1, 0));
    assert_eq!(e.st(video, "stt"), "queued");
    assert_eq!(steps(&e), 1, "자막 한 단계만");
    assert_eq!(origin_of(&e, video, "stt").as_deref(), Some("auto_new"));
    assert!(e.ext.calls.lock().unwrap().is_empty(), "외부 전송 없음");
}

#[test]
fn t_subtitle_02_auto_analyze_only_never_queues_stt() {
    // Arrange
    let e = AiEnv::new();
    analyze_only(&e);
    let video = e.add_asset("video");
    let audio = e.add_asset("audio");

    // Act
    let rv = e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();
    let ra = e.sched.enqueue_new_asset(audio, &snap(&e)).unwrap();

    // Assert: 영상은 외부 종류만, 음성은 외부 대상이 아니라 아무것도 등록되지 않는다
    assert_eq!(rv.queued, 1);
    assert_eq!(ra.queued, 0);
    assert_eq!(e.st(video, "describe"), "queued");
    assert_eq!(e.st(video, "stt"), "none");
    assert_eq!(e.st(audio, "stt"), "none");
}

#[test]
fn t_subtitle_03_both_options_merge_into_one_job_with_merged_kinds() {
    // Arrange
    let e = AiEnv::new();
    analyze_only(&e);
    e.set(|s| {
        s.auto_subtitle_new = true;
        s.semantic_enabled = true;
    });
    let video = e.add_asset("video");

    // Act
    let r = e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();

    // Assert
    assert_eq!(r.queued, 1);
    assert_eq!(jobs(&e), 1, "자산당 작업 1개");
    for k in ["describe", "keywords", "ocr", "embedding", "stt"] {
        assert_eq!(e.st(video, k), "queued", "{k}");
        assert_eq!(origin_of(&e, video, k).as_deref(), Some("auto_new"));
    }
}

#[test]
fn t_subtitle_04_stt_switch_off_or_image_registers_nothing() {
    // Arrange: 자막 스위치 꺼짐 + 이미지
    let e = AiEnv::new();
    subtitle_only(&e);
    let video = e.add_asset("video");
    let image = e.add_asset("image");
    e.set(|s| s.stt_enabled = false);

    // Act
    let off = e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();
    e.set(|s| s.stt_enabled = true);
    let img = e.sched.enqueue_new_asset(image, &snap(&e)).unwrap();

    // Assert
    assert_eq!((off.queued, off.skipped), (0, 1));
    assert_eq!((img.queued, img.skipped), (0, 1));
    assert_eq!((steps(&e), jobs(&e)), (0, 0));
}

#[test]
fn t_subtitle_05_both_options_off_registers_nothing() {
    let e = AiEnv::new();
    e.set(|s| {
        s.consent_given = true;
        s.auto_analyze_new = false;
        s.auto_subtitle_new = false;
    });
    let video = e.add_asset("video");
    let r = e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();
    assert_eq!((r.queued, r.skipped), (0, 1));
    assert_eq!(steps(&e), 0);
}

#[test]
fn t_subtitle_06_stt_only_new_video_is_not_blocked_by_missing_frame_strip() {
    // Arrange: 프레임 스트립이 아직 없는 신규 영상 + 자막만 등록
    let e = AiEnv::new();
    subtitle_only(&e);
    let video = e.add_asset("video");
    e.set_strip(video, None, false);
    e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();

    // Act
    let out = e.run_next(&|| false);

    // Assert: 기다리지 않고 바로 실행 완료
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(video, "stt"), "done");
    assert_eq!(e.stt.calls.lock().unwrap().len(), 1);
    assert!(e.ext.calls.lock().unwrap().is_empty());
}

// ── B8. 종류별 정리 ─────────────────────────────────────────────────────

/// 영상 1개에 자동 분석(describe 등)과 자막이 모두 auto_new 로 등록된 상태.
fn both_registered() -> (AiEnv, i64) {
    let e = AiEnv::new();
    analyze_only(&e);
    e.set(|s| s.auto_subtitle_new = true);
    let video = e.add_asset("video");
    e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();
    (e, video)
}

const ONLY_STT: OriginScope = OriginScope {
    origin: Origin::AutoNew,
    kinds: KindScope::OnlyStt,
};
const EXCEPT_STT: OriginScope = OriginScope {
    origin: Origin::AutoNew,
    kinds: KindScope::ExceptStt,
};

#[test]
fn t_subtitle_07_subtitle_off_deletes_only_queued_auto_new_stt() {
    // Arrange
    let (e, video) = both_registered();

    // Act
    let n = e.sched.cancel_auto_scopes(&[ONLY_STT]).unwrap();

    // Assert: 자막만 사라지고 작업은 남은 단계 때문에 보존된다
    assert_eq!(n, 1);
    assert_eq!(e.st(video, "stt"), "none");
    assert_eq!(e.st(video, "describe"), "queued");
    assert_eq!(jobs(&e), 1);
}

#[test]
fn t_subtitle_08_analyze_off_keeps_stt_and_stt_only_asset_keeps_its_job() {
    // Arrange
    let (e, video) = both_registered();

    // Act
    let n = e.sched.cancel_auto_scopes(&[EXCEPT_STT]).unwrap();

    // Assert
    assert!(n >= 3);
    assert_eq!(e.st(video, "stt"), "queued");
    assert_eq!(e.st(video, "describe"), "none");
    assert_eq!(jobs(&e), 1);

    // 자막만 남은 자산에서 자막까지 지우면 대기 작업도 사라진다
    e.sched.cancel_auto_scopes(&[ONLY_STT]).unwrap();
    assert_eq!((steps(&e), jobs(&e)), (0, 0));
}

#[test]
fn t_subtitle_09_running_manual_and_legacy_null_rows_are_untouched_and_cleanup_is_idempotent() {
    // Arrange: a=running auto_new stt, b=manual stt, c=NULL 출처 stt, d=queued auto_new stt
    let e = AiEnv::new();
    let ids: Vec<i64> = (0..4).map(|_| e.add_asset("video")).collect();
    put_step(&e, ids[0], "stt", "running", "auto_new");
    put_step(&e, ids[1], "stt", "queued", "manual");
    put_step(&e, ids[2], "stt", "queued", "manual");
    exec(
        &e,
        &format!(
            "UPDATE ai_task_state SET origin = NULL WHERE asset_id = {}",
            ids[2]
        ),
    );
    put_step(&e, ids[3], "stt", "queued", "auto_new");

    // Act
    let first = e.sched.cancel_auto_scopes(&[ONLY_STT]).unwrap();
    let second = e.sched.cancel_auto_scopes(&[ONLY_STT]).unwrap();
    let empty = e.sched.cancel_auto_scopes(&[]).unwrap();

    // Assert
    assert_eq!((first, second, empty), (1, 0, 0));
    assert_eq!(e.st(ids[0], "stt"), "running");
    assert_eq!(e.st(ids[1], "stt"), "queued");
    assert_eq!(e.st(ids[2], "stt"), "queued");
    assert_eq!(e.st(ids[3], "stt"), "none");
}

#[test]
fn t_subtitle_10_merged_scopes_equal_all_and_cancel_auto_pending_still_means_all_kinds() {
    // Arrange
    let (e, _) = both_registered();

    // Act: OnlyStt + ExceptStt = 전체
    let n = e.sched.cancel_auto_scopes(&[ONLY_STT, EXCEPT_STT]).unwrap();

    // Assert
    assert!(n >= 4);
    assert_eq!((steps(&e), jobs(&e)), (0, 0));

    // cancel_auto_pending(&[AutoNew]) 도 종류 구분 없이 지운다 (기존 계약)
    let (e2, _) = both_registered();
    e2.sched.cancel_auto_pending(&[Origin::AutoNew]).unwrap();
    assert_eq!((steps(&e2), jobs(&e2)), (0, 0));
}

#[test]
fn t_subtitle_11_executor_gate_drops_auto_new_stt_when_subtitle_is_off_but_keeps_other_kinds() {
    // Arrange: 자막만 등록된 영상 + 자동 분석 단계가 등록된 영상, 그 뒤 자동 자막만 끈다
    let e = AiEnv::new();
    subtitle_only(&e);
    let a = e.add_asset("video");
    e.sched.enqueue_new_asset(a, &snap(&e)).unwrap();
    analyze_only(&e);
    let b = e.add_asset("video");
    e.sched.enqueue_new_asset(b, &snap(&e)).unwrap();
    e.set(|s| s.auto_subtitle_new = false); // 자동 분석은 켠 채 자막만 꺼짐

    // Act
    while e.run_next(&|| false).is_some() {}

    // Assert: a 의 자막은 실행되지 않고 행이 지워지며, b 의 외부 분석은 실행된다
    assert!(e.stt.calls.lock().unwrap().is_empty());
    assert_eq!(e.st(a, "stt"), "none");
    assert_eq!(e.st(b, "describe"), "done");
}

#[test]
fn t_subtitle_12_executor_gate_drops_analysis_but_runs_stt_when_only_analyze_is_off() {
    // Arrange: 둘 다 등록 후 자동 분석만 끈다 (자막 실효값은 명시 true)
    let (e, video) = both_registered();
    e.set(|s| s.auto_analyze_new = false);

    // Act
    while e.run_next(&|| false).is_some() {}

    // Assert
    assert!(
        e.ext.calls.lock().unwrap().is_empty(),
        "외부 분석은 실행되지 않는다"
    );
    assert_eq!(e.st(video, "describe"), "none");
    assert_eq!(e.st(video, "stt"), "done");
}

#[test]
fn t_subtitle_13_executor_gate_after_restart_recover_drops_running_auto_stt() {
    // Arrange: 실행 중이던 auto_new 자막 -> 자막 옵션을 끈 채 재시작
    let e = AiEnv::new();
    subtitle_only(&e);
    let video = e.add_asset("video");
    e.sched.enqueue_new_asset(video, &snap(&e)).unwrap();
    e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    exec(
        &e,
        "UPDATE ai_task_state SET state = 'running' WHERE kind = 'stt'",
    );
    e.set(|s| s.auto_subtitle_new = false);
    let q = mam_core::jobs::JobQueue::with_concurrency(
        std::sync::Arc::clone(&e.env.pool),
        e.env.clock.clone(),
        AiJobsConfig::new(e.cache.clone()).queue_caps(),
    );
    assert_eq!(q.recover().unwrap(), 1);

    // Act
    while let Some(job) = q.claim_next(&[JobKind::Ai]).unwrap() {
        let res = e.exec.run(&job, &|| false);
        q.finish(&job, res).unwrap();
    }

    // Assert
    assert!(e.stt.calls.lock().unwrap().is_empty());
    assert_eq!(steps(&e), 0);
}

// ── C10. 자막 일괄 생성 ─────────────────────────────────────────────────

#[test]
fn t_subtitle_20_selection_includes_missing_skipped_failed_and_excludes_the_rest() {
    // Arrange
    let e = AiEnv::new();
    let none = e.add_asset("video");
    let skipped = e.add_asset("audio");
    let failed = e.add_asset("video");
    let done = e.add_asset("video");
    let queued = e.add_asset("video");
    let running = e.add_asset("audio");
    let image = e.add_asset("image");
    let superseded = e.add_asset("video");
    let deleted = e.add_asset("video");
    let only_desc = e.add_asset("video"); // 설명은 끝났지만 자막 행이 없다
    put_step(&e, skipped, "stt", "skipped", "manual");
    put_step(&e, failed, "stt", "failed", "manual");
    put_step(&e, done, "stt", "done", "manual");
    put_step(&e, queued, "stt", "queued", "manual");
    put_step(&e, running, "stt", "running", "manual");
    put_step(&e, only_desc, "describe", "done", "manual");
    exec(
        &e,
        &format!("UPDATE media_assets SET status='superseded' WHERE asset_id={superseded}"),
    );
    exec(
        &e,
        &format!("UPDATE media_assets SET status='deleted' WHERE asset_id={deleted}"),
    );

    // Act
    let r = e.sched.generate_subtitles_all(&snap(&e)).unwrap();

    // Assert: 없음·skipped·failed·설명만 있는 자산 = 4
    assert_eq!((r.queued, r.skipped), (4, 0));
    for id in [none, skipped, failed, only_desc] {
        assert_eq!(e.st(id, "stt"), "queued", "asset {id}");
        assert_eq!(origin_of(&e, id, "stt").as_deref(), Some("manual"));
    }
    assert_eq!(e.st(done, "stt"), "done");
    assert_eq!(e.st(running, "stt"), "running");
    for id in [image, superseded, deleted] {
        assert_eq!(e.st(id, "stt"), "none", "asset {id}");
    }
    assert_eq!(jobs(&e), 4);
}

#[test]
fn t_subtitle_21_running_twice_adds_nothing_and_does_not_need_consent() {
    // Arrange: 동의 없음
    let e = AiEnv::new();
    e.set(|s| s.consent_given = false);
    e.add_asset("video");
    e.add_asset("audio");

    // Act
    let first = e.sched.generate_subtitles_all(&snap(&e)).unwrap();
    let second = e.sched.generate_subtitles_all(&snap(&e)).unwrap();

    // Assert
    assert_eq!(first.queued, 2);
    assert_eq!((second.queued, second.skipped), (0, 0));
    assert_eq!((steps(&e), jobs(&e)), (2, 2));
}

#[test]
fn t_subtitle_22_disabled_switch_is_rejected_without_side_effects() {
    // Arrange
    let e = AiEnv::new();
    e.add_asset("video");
    e.set(|s| s.stt_enabled = false);

    // Act
    let err = e.sched.generate_subtitles_all(&snap(&e)).unwrap_err();

    // Assert
    assert!(matches!(err, AiJobError::SubtitleDisabled));
    assert_eq!((steps(&e), jobs(&e)), (0, 0));
}

#[test]
fn t_subtitle_23_pages_of_500_cover_every_asset_exactly_once() {
    for total in [500_usize, 501, 1001] {
        // Arrange
        let e = AiEnv::new();
        e.w(move |c, _| {
            for i in 0..total {
                let media = if i % 2 == 0 { "video" } else { "audio" };
                c.execute(
                    "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                     VALUES (?1, 1, 1, 1, 1)",
                    [media],
                )
                .unwrap();
            }
        });

        // Act
        let r = e.sched.generate_subtitles_all(&snap(&e)).unwrap();
        let again = e.sched.generate_subtitles_all(&snap(&e)).unwrap();

        // Assert
        assert_eq!(r.queued as usize, total, "total={total}");
        assert_eq!(again.queued, 0);
        assert_eq!(
            e.env
                .count("SELECT COUNT(*) FROM ai_task_state WHERE kind = 'stt' AND state = 'queued'")
                as usize,
            total
        );
    }
}

#[test]
fn t_subtitle_24_query_plan_uses_the_state_primary_key_without_temp_btree() {
    // Arrange
    let e = AiEnv::new();
    e.add_asset("video");

    // Act
    let plan = e.env.read(|c| {
        let mut s = c
            .prepare(&format!("EXPLAIN QUERY PLAN {MISSING_SUBTITLE_SQL}"))
            .unwrap();
        s.query_map([0i64, 10], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    });
    println!("EXPLAIN: {plan}");

    // Assert
    assert!(
        plan.contains("SEARCH s USING")
            && !plan.contains("SCAN s")
            && !plan.contains("TEMP B-TREE"),
        "{plan}"
    );
}
