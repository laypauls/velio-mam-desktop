//! T-aioff-01~08 - 자동 옵션을 꺼도 이미 등록된 AI 작업이 계속되던 결함의 회귀 시험 (출처 기록·정리·실행 게이트).
//! 분류: Happy / Boundary / Business rule / Concurrency(실행 중 보존·멱등).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::*;
use mam_core::jobs::JobKind;

const SWEEP_OFFSET: i32 = 0;

fn snap(e: &AiEnv) -> AiRuntimeSettings {
    e.settings.snapshot()
}

fn exec(e: &AiEnv, sql: &str) {
    let sql = sql.to_string();
    e.w(move |c, _| c.execute(&sql, []).unwrap());
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

fn pending_jobs(e: &AiEnv) -> i64 {
    e.env
        .count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'")
}

fn step_rows(e: &AiEnv) -> i64 {
    e.env.count("SELECT COUNT(*) FROM ai_task_state")
}

/// 백그라운드 분석 ON(예약 없음)인 환경과 이미지 자산들.
fn bg_env(assets: usize) -> (AiEnv, Vec<i64>) {
    let e = AiEnv::new();
    let ids = (0..assets).map(|_| e.add_asset("image")).collect();
    e.set(|s| s.background_analyze = true);
    (e, ids)
}

#[test]
fn t_aioff_01_turning_background_off_deletes_its_queued_steps_and_pending_jobs() {
    // Arrange
    let (e, _) = bg_env(3);
    let queued = e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    assert_eq!(queued.queued, 3);
    assert!(step_rows(&e) > 0 && pending_jobs(&e) == 3);

    // Act
    let removed = e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();

    // Assert
    assert!(removed > 0);
    assert_eq!(step_rows(&e), 0);
    assert_eq!(pending_jobs(&e), 0);
}

#[test]
fn t_aioff_02_manual_requests_survive_the_cleanup_and_still_run() {
    // Arrange: 자산 0 은 사용자가 직접 요청, 나머지는 스윕
    let (e, ids) = bg_env(3);
    e.sched
        .analyze_assets(&[ids[0]], Some(&[StepKind::Describe]), false, &snap(&e))
        .unwrap();
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    e.set(|s| s.background_analyze = false);

    // Act
    e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();

    // Assert: manual 행·작업은 그대로, 스윕분은 정리
    assert_eq!(e.st(ids[0], "describe"), "queued");
    assert_eq!(e.st(ids[1], "describe"), "none");
    assert_eq!(pending_jobs(&e), 1);
    e.run_next(&|| false).unwrap().unwrap();
    assert_eq!(e.st(ids[0], "describe"), "done");
    assert_eq!(e.ext.calls.lock().unwrap().len(), 1);
}

#[test]
fn t_aioff_03_turning_background_on_again_reselects_the_cleaned_assets() {
    // Arrange
    let (e, ids) = bg_env(3);
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();

    // Act
    let again = e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();

    // Assert
    assert_eq!(again.queued, 3);
    assert_eq!(
        origin_of(&e, ids[0], "describe").as_deref(),
        Some("background")
    );
}

#[test]
fn t_aioff_04_executor_gate_skips_disabled_origin_after_restart_recover() {
    // Arrange: 작업이 실행 중(단계 running)이던 상태에서 비정상 종료 -> 옵션을 끈 채 재시작
    let (e, _) = bg_env(2);
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    exec(
        &e,
        "UPDATE ai_task_state SET state = 'running' WHERE kind = 'describe'",
    );
    e.set(|s| s.background_analyze = false);
    // 재시작 = 새 대기열 인스턴스(메모리상 실행 중 계수 없음) + recover
    let q = mam_core::jobs::JobQueue::with_concurrency(
        std::sync::Arc::clone(&e.env.pool),
        e.env.clock.clone(),
        AiJobsConfig::new(e.cache.clone()).queue_caps(),
    );
    assert_eq!(q.recover().unwrap(), 1);

    // Act: 정리 호출 없이 실행기만 돈다
    while let Some(job) = q.claim_next(&[JobKind::Ai]).unwrap() {
        let res = e.exec.run(&job, &|| false);
        q.finish(&job, res).unwrap();
    }

    // Assert: 어떤 러너도 호출되지 않고 행이 남지 않는다
    assert!(e.stt.calls.lock().unwrap().is_empty());
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert!(e.emb.calls.lock().unwrap().is_empty());
    assert_eq!(step_rows(&e), 0);
    // 다시 켜면 같은 자산이 다시 선택된다
    e.set(|s| s.background_analyze = true);
    assert_eq!(
        e.sched
            .background_sweep(&snap(&e), SWEEP_OFFSET)
            .unwrap()
            .queued,
        2
    );
}

#[test]
fn t_aioff_04b_executor_gate_also_covers_consent_revoked_and_backfill() {
    // Arrange: 신규 자동(auto_new)은 동의가 필요하고, 백필은 의미 검색이 켜져 있어야 한다
    let e = AiEnv::new();
    let (a, b) = (e.add_asset("image"), e.add_asset("image"));
    e.set(|s| s.auto_analyze_new = true);
    e.sched.enqueue_new_asset(a, &snap(&e)).unwrap();
    // a 는 이미 행이 있어 백필 대상에서 제외되고, b 만 backfill 출처로 등록된다
    e.sched.embedding_backfill(&snap(&e)).unwrap();
    assert_eq!(origin_of(&e, b, "embedding").as_deref(), Some("backfill"));
    e.set(|s| {
        s.consent_given = false;
        s.semantic_enabled = false;
    });

    // Act
    while e.run_next(&|| false).is_some() {}

    // Assert
    assert!(e.stt.calls.lock().unwrap().is_empty());
    assert!(e.emb.calls.lock().unwrap().is_empty());
    assert_eq!(step_rows(&e), 0);
}

#[test]
fn t_aioff_05_running_steps_are_not_cleaned_by_the_settings_cleanup() {
    // Arrange: 진행 중(running)인 background 단계가 있다
    let (e, ids) = bg_env(1);
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    exec(
        &e,
        "UPDATE ai_task_state SET state = 'running' WHERE kind = 'describe'",
    );

    // Act
    e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();

    // Assert: running 행은 남고 나머지 queued 단계는 지워지며, 처리할 단계가 남은 자산의 작업은 보존된다
    assert_eq!(e.st(ids[0], "describe"), "running");
    assert_eq!(step_rows(&e), 1);
    assert_eq!(pending_jobs(&e), 1);
}

#[test]
fn t_aioff_06_cleanup_is_idempotent_and_legacy_null_origin_rows_count_as_manual() {
    // Arrange: 구 행(origin NULL) 하나 + 스윕 하나
    let (e, ids) = bg_env(2);
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    exec(
        &e,
        &format!(
            "UPDATE ai_task_state SET origin = NULL WHERE asset_id = {}",
            ids[0]
        ),
    );

    // Act
    let first = e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();
    let second = e.sched.cancel_auto_pending(&[Origin::Background]).unwrap();
    let none = e.sched.cancel_auto_pending(&[]).unwrap();

    // Assert
    assert!(first > 0);
    assert_eq!((second, none), (0, 0));
    assert_eq!(e.st(ids[0], "describe"), "queued");
    assert_eq!(e.st(ids[1], "describe"), "none");
    // 구 행은 실행기 게이트도 통과해 실행된다
    e.set(|s| s.background_analyze = false);
    e.run_next(&|| false).unwrap().unwrap();
    assert_eq!(e.st(ids[0], "describe"), "done");
}

#[test]
fn t_aioff_07_manual_request_promotes_an_auto_queued_row_and_auto_never_downgrades() {
    // Arrange: 스윕 행 -> 같은 자산에 사용자가 수동 요청 (강제 아님)
    let (e, ids) = bg_env(1);
    e.sched.background_sweep(&snap(&e), SWEEP_OFFSET).unwrap();
    assert_eq!(
        origin_of(&e, ids[0], "describe").as_deref(),
        Some("background")
    );

    // Act
    e.sched
        .analyze_assets(&[ids[0]], None, false, &snap(&e))
        .unwrap();
    // 자동 경로가 다시 덮어도 강등되지 않는다
    e.set(|s| s.auto_analyze_new = true);
    e.sched.enqueue_new_asset(ids[0], &snap(&e)).unwrap();
    e.set(|s| s.background_analyze = false);
    e.sched
        .cancel_auto_pending(&[Origin::Background, Origin::AutoNew])
        .unwrap();

    // Assert
    assert_eq!(origin_of(&e, ids[0], "describe").as_deref(), Some("manual"));
    assert_eq!(e.st(ids[0], "describe"), "queued");
    assert_eq!(pending_jobs(&e), 1);
}

#[test]
fn t_aioff_08_origin_is_recorded_per_path_and_retry_and_stale_requeue_are_manual() {
    // Arrange
    let e = AiEnv::new();
    let (a, b, c) = (
        e.add_asset("image"),
        e.add_asset("image"),
        e.add_asset("image"),
    );
    e.set(|s| s.auto_analyze_new = true);

    // Act
    e.sched.enqueue_new_asset(a, &snap(&e)).unwrap();
    e.sched
        .analyze_assets(&[b], Some(&[StepKind::Describe]), false, &snap(&e))
        .unwrap();
    exec(
        &e,
        &format!(
            "UPDATE ai_task_state SET state = 'failed', origin = 'auto_new' WHERE asset_id = {b}"
        ),
    );
    e.sched.retry_failed(Some(&[b])).unwrap();
    exec(
        &e,
        &format!(
            "INSERT INTO ai_task_state (asset_id, kind, state, attempts, updated_at, origin)
             VALUES ({c}, 'embedding', 'done', 0, 1, 'backfill')"
        ),
    );
    e.w(move |conn, clock| {
        mam_core::ai_store::requeue_semantic_embedding(conn, clock.now_ms(), c).unwrap()
    });

    // Assert
    assert_eq!(origin_of(&e, a, "describe").as_deref(), Some("auto_new"));
    assert_eq!(origin_of(&e, b, "describe").as_deref(), Some("manual"));
    assert_eq!(origin_of(&e, c, "embedding").as_deref(), Some("manual"));
}

#[test]
fn t_aioff_11_legacy_backfilled_background_row_is_dropped_by_the_gate_and_reselected_by_backfill() {
    // Arrange: V28 이 구 대기 행을 background 로 표시한 상태 (수동 등록 후 출처를 덮어 재현), 옵션은 꺼짐·의미 검색은 켜짐
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.sched
        .analyze_assets(&[id], None, false, &snap(&e))
        .unwrap();
    exec(&e, "UPDATE ai_task_state SET origin = 'background'");
    assert_eq!(
        origin_of(&e, id, "embedding").as_deref(),
        Some("background")
    );
    e.set(|s| s.background_analyze = false);

    // Act: 실행기 게이트
    while e.run_next(&|| false).is_some() {}

    // Assert: 러너 호출 없이 행이 지워진다
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert!(e.emb.calls.lock().unwrap().is_empty());
    assert_eq!(step_rows(&e), 0);

    // Act: 의미 검색이 켜져 있으면 임베딩 백필이 같은 자산을 다시 집는다 (자가 치유)
    let again = e.sched.embedding_backfill(&snap(&e)).unwrap();

    // Assert
    assert_eq!(again.queued, 1);
    assert_eq!(origin_of(&e, id, "embedding").as_deref(), Some("backfill"));
}
