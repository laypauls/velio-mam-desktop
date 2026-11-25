//! T-backfill-01~08 - 의미 검색 켜짐 상태의 임베딩 백필 (`AiScheduler::embedding_backfill`).
//! 분류: Happy / Boundary(예산 0·대기 초과·배치 상한) / Business rule(동의 무관·외부 단계 미등록·기존 행·비활성 제외) /
//! Concurrency(멱등·커서) / Validation(쿼리 계획).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use std::sync::Arc;

use ai_env::AiEnv;
use mam_core::SharedClock;
use mam_core::ai_jobs::*;

fn jobs(e: &AiEnv) -> i64 {
    e.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai'")
}

fn snap(e: &AiEnv) -> AiRuntimeSettings {
    e.settings.snapshot()
}

fn set_embedding_row(e: &AiEnv, id: i64, state: &'static str) {
    e.w(move |c, _| {
        c.execute(
            "INSERT INTO ai_task_state (asset_id, kind, state, error_code, attempts, updated_at)
             VALUES (?1, 'embedding', ?2, NULL, 0, 1)",
            rusqlite::params![id, state],
        )
        .unwrap();
    });
}

#[test]
fn t_backfill_01_off_queues_nothing() {
    // Arrange
    let e = AiEnv::new();
    e.add_asset("image");
    e.set(|s| s.semantic_enabled = false);

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(r, QueueResult::default());
    assert_eq!(jobs(&e), 0);
    assert_eq!(e.env.count("SELECT COUNT(*) FROM ai_task_state"), 0);
}

#[test]
fn t_backfill_02_on_queues_only_embedding_for_image_video_audio() {
    // Arrange
    let e = AiEnv::new();
    let ids = [
        e.add_asset("image"),
        e.add_asset("video"),
        e.add_asset("audio"),
    ];

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert: 임베딩만 (외부 4종·stt 는 행 자체가 없다)
    assert_eq!(r.queued, 3);
    assert_eq!(jobs(&e), 3);
    for id in ids {
        assert_eq!(e.st(id, "embedding"), "queued");
        for k in ["describe", "keywords", "ocr", "summary", "stt"] {
            assert_eq!(e.st(id, k), "none", "asset {id} {k}");
        }
    }
}

#[test]
fn t_backfill_03_runs_without_consent_and_without_external_kinds() {
    // Arrange: 동의 없음, 외부 종류 비어 있음, 백그라운드·자동 분석 꺼짐, 예약 시간대 밖
    let e = AiEnv::new();
    let img = e.add_asset("image");
    e.set(|s| {
        s.consent_given = false;
        s.external_kinds.clear();
        s.background_analyze = false;
        s.auto_analyze_new = false;
        s.schedule = Schedule {
            enabled: true,
            start_hour: 9,
            end_hour: 10,
        };
    });

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(r.queued, 1);
    assert_eq!(e.st(img, "embedding"), "queued");
}

#[test]
fn t_backfill_04_excludes_assets_with_any_embedding_row() {
    // Arrange: done / failed / skipped 는 다시 고르지 않는다
    let e = AiEnv::new();
    let (done, failed, skipped, fresh) = (
        e.add_asset("image"),
        e.add_asset("image"),
        e.add_asset("video"),
        e.add_asset("image"),
    );
    set_embedding_row(&e, done, "done");
    set_embedding_row(&e, failed, "failed");
    set_embedding_row(&e, skipped, "skipped");

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(r.queued, 1);
    assert_eq!(e.st(fresh, "embedding"), "queued");
    assert_eq!(e.st(done, "embedding"), "done");
    assert_eq!(e.st(failed, "embedding"), "failed");
    assert_eq!(e.st(skipped, "embedding"), "skipped");
    assert_eq!(jobs(&e), 1);
}

#[test]
fn t_backfill_05_other_step_rows_do_not_exclude_but_inactive_assets_do() {
    // Arrange: 외부 분석만 끝난 자산(배경 4)은 대상, 비활성 자산은 제외
    let e = AiEnv::new();
    let analyzed = e.add_asset("image");
    e.w(move |c, _| {
        c.execute(
            "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at)
             VALUES (?1, 'describe', 'done', 0, 1)",
            [analyzed],
        )
        .unwrap();
    });
    let gone = e.add_asset("image");
    let old = e.add_asset("image");
    e.w(move |c, _| {
        c.execute(
            "UPDATE media_assets SET status = 'deleted' WHERE asset_id = ?1",
            [gone],
        )
        .unwrap();
        c.execute(
            "UPDATE media_assets SET status = 'superseded' WHERE asset_id = ?1",
            [old],
        )
        .unwrap();
    });

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(r.queued, 1);
    assert_eq!(e.st(analyzed, "embedding"), "queued");
    assert_eq!(e.st(gone, "embedding"), "none");
    assert_eq!(e.st(old, "embedding"), "none");
}

#[test]
fn t_backfill_06_budget_is_batch_minus_pending_and_zero_budget_queues_nothing() {
    // Arrange: 배치 3, 자산 5
    let e = AiEnv::new();
    let ids: Vec<i64> = (0..5).map(|_| e.add_asset("image")).collect();
    let clock: SharedClock = e.env.clock.clone();
    let sched = AiScheduler::new(Arc::clone(&e.env.pool), Arc::clone(&e.queue), clock, 3);

    // Act / Assert: 상한 3, 대기가 가득이면 0 (Boundary)
    assert_eq!(sched.embedding_backfill(&snap(&e)).unwrap().queued, 3);
    assert_eq!(jobs(&e), 3);
    assert_eq!(
        sched.embedding_backfill(&snap(&e)).unwrap(),
        QueueResult::default(),
        "대기 작업이 배치 이상"
    );
    // 하나 시작하면 한 건만 더, 그 다음 호출은 남은 하나가 아니라 커서 뒤(남은 자산)부터
    e.queue
        .claim_next(&[mam_core::jobs::JobKind::Ai])
        .unwrap()
        .unwrap();
    assert_eq!(sched.embedding_backfill(&snap(&e)).unwrap().queued, 1);
    assert_eq!(e.st(ids[3], "embedding"), "queued");
    assert_eq!(e.st(ids[4], "embedding"), "none");

    // 배치 0 설정 자체도 아무것도 하지 않는다
    let zero = AiScheduler::new(
        Arc::clone(&e.env.pool),
        Arc::clone(&e.queue),
        e.env.clock.clone(),
        0,
    );
    assert_eq!(
        zero.embedding_backfill(&snap(&e)).unwrap(),
        QueueResult::default()
    );
}

#[test]
fn t_backfill_07_is_idempotent_and_picks_up_new_assets() {
    // Arrange
    let e = AiEnv::new();
    e.add_asset("image");
    e.add_asset("audio");

    // Act
    let first = e.sched.embedding_backfill(&snap(&e)).unwrap();
    let second = e.sched.embedding_backfill(&snap(&e)).unwrap();
    let late = e.add_asset("video");
    let third = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert: 중복 등록 없음, 나중 자산은 다음 틱에 등록
    assert_eq!(first.queued, 2);
    assert_eq!(second, QueueResult::default());
    assert_eq!(third.queued, 1);
    assert_eq!(e.st(late, "embedding"), "queued");
    assert_eq!(jobs(&e), 3);
}

#[test]
fn t_backfill_08_toggle_off_resets_cursor_and_query_plan_uses_primary_key() {
    // Arrange: 설정 끔 -> 커서 초기화, 다시 켜면 행이 없어진 앞쪽 자산도 대상
    let e = AiEnv::new();
    let a = e.add_asset("image");
    e.add_asset("image");
    assert_eq!(e.sched.embedding_backfill(&snap(&e)).unwrap().queued, 2);
    e.w(move |c, _| {
        c.execute("DELETE FROM ai_task_state WHERE asset_id = ?1", [a])
            .unwrap();
        c.execute("DELETE FROM jobs WHERE asset_id = ?1", [a])
            .unwrap();
    });
    assert_eq!(
        e.sched.embedding_backfill(&snap(&e)).unwrap().queued,
        0,
        "커서 뒤에는 대상이 없다"
    );
    e.set(|s| s.semantic_enabled = false);
    e.sched.embedding_backfill(&snap(&e)).unwrap();
    e.set(|s| s.semantic_enabled = true);

    // Act
    let r = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(r.queued, 1);
    assert_eq!(e.st(a, "embedding"), "queued");
    let plan = e.env.read(|c| {
        let mut s = c
            .prepare(&format!("EXPLAIN QUERY PLAN {EMBEDDING_BACKFILL_SQL}"))
            .unwrap();
        s.query_map([0i64, 10], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    });
    println!("EXPLAIN: {plan}");
    assert!(
        plan.contains("SEARCH s USING")
            && !plan.contains("SCAN s")
            && !plan.contains("TEMP B-TREE"),
        "{plan}"
    );
}
