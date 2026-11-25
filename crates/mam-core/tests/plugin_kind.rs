//! `JobKind::Plugin` 계약 (T-pjob-26) — 정의 상수·일시정지 비트·대기열이 plugin 행을 다루는 방식 (구 `plugin_kind_ignored.rs` 대체).
//!
//! 분류: Boundary (정의 상수·비트) / Business rule (일시정지 영속화·알 수 없는 종류 무시·정리) / Concurrency (재시작 복구).
//! 실제 SQLite + 수동 시계.

mod common;

use std::sync::Arc;

use common::env::Env;
use common::purge::{exec, insert_assets};
use mam_core::SharedClock;
use mam_core::jobs::{JobKind, JobQueue, NewJob};
use mam_db::Priority;
use mam_db::job_pause::{JobPauseState, load_job_pause, save_job_pause};

fn queue(env: &Env) -> Arc<JobQueue> {
    let clock: SharedClock = env.clock.clone();
    JobQueue::new(Arc::clone(&env.pool), clock)
}

/// 같은 DB 위의 새 큐 — 시작 시 복구·일시정지 복원
fn restarted(env: &Env) -> Arc<JobQueue> {
    let q = queue(env);
    q.recover().unwrap();
    q.restore_pause().unwrap();
    q
}

fn env_with_volume() -> Env {
    let env = Env::new();
    std::fs::create_dir_all(env.path("lib")).unwrap();
    env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap();
    env
}

#[test]
fn t_pjob_26_kind_definition() {
    // Arrange / Act / Assert
    assert_eq!(JobKind::COUNT, 8);
    assert_eq!(JobKind::ALL.len(), 8);
    assert!(JobKind::ALL.contains(&JobKind::Plugin));
    assert_eq!(JobKind::Plugin.as_str(), "plugin");
    assert_eq!(JobKind::Plugin.as_str(), mam_db::plugin_tasks::JOB_KIND);
    assert_eq!(JobKind::parse("plugin"), Some(JobKind::Plugin));
    assert_eq!(JobKind::Plugin.default_priority(), 70);
    assert_eq!(JobKind::Plugin.default_concurrency(), 1);
    // 일시정지 마스크(AtomicU8)의 마지막 비트 — 다른 종류와 겹치지 않는다
    assert_eq!(JobKind::ALL.iter().position(|k| *k == JobKind::Plugin), Some(7));
    assert_eq!(serde_json::to_value(JobKind::Plugin).unwrap(), "plugin");
    let lowest = JobKind::ALL.iter().map(|k| k.default_priority()).max();
    assert_eq!(lowest, Some(70), "plugin runs after every other kind");
}

#[test]
fn t_pjob_26_pause_bit_survives_save_and_restore() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);

    // Act: 종류만 일시정지 -> 저장 -> 재시작
    q.pause(Some(JobKind::Plugin)).unwrap();
    let saved = env.pool.read(load_job_pause).unwrap();
    let q2 = restarted(&env);

    // Assert: 저장은 이름 `plugin` 만, 복원 뒤에도 그 종류만 멈춰 있다
    assert_eq!(saved, JobPauseState { all: false, kinds: vec!["plugin".to_owned()] });
    assert_eq!(q2.paused_kinds(), vec![JobKind::Plugin]);
    assert!(q2.is_paused(JobKind::Plugin));
    assert!(JobKind::ALL.into_iter().filter(|k| *k != JobKind::Plugin).all(|k| !q2.is_paused(k)));
    // 재개하면 비트가 지워지고 그 상태도 저장된다
    q2.resume(Some(JobKind::Plugin)).unwrap();
    assert!(!restarted(&env).is_paused(JobKind::Plugin));
}

#[test]
fn t_pjob_26_unknown_kind_names_in_saved_pause_are_ignored() {
    // Arrange: 이후 버전이 저장한 모르는 종류 이름이 섞여 있다
    let env = Env::new();
    env.pool
        .write(Priority::High, |c| {
            save_job_pause(c, &JobPauseState { all: false, kinds: vec!["plugin".into(), "from-the-future".into()] })?;
            Ok(())
        })
        .unwrap();

    // Act
    let q = restarted(&env);

    // Assert: 알려진 종류만 복원된다 (오류 없음)
    assert_eq!(q.paused_kinds(), vec![JobKind::Plugin]);
}

#[test]
fn t_pjob_26_plugin_rows_are_claimed_one_at_a_time_after_other_kinds() {
    // Arrange: 상태별 plugin 행 + 일반 작업 1건 — 재시작 복구는 running 을 pending 으로 돌린다
    let env = Env::new();
    exec(
        &env,
        "INSERT INTO jobs (kind, asset_id, priority, status, created_at, updated_at) VALUES
           ('plugin', 9001, 70, 'pending', 1, 1), ('plugin', 9002, 70, 'running', 1, 1), ('plugin', 9003, 70, 'failed', 1, 1);",
    );
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 5)).unwrap();

    // Act
    assert!(q.recover().is_ok());
    let first = q.claim_next(&JobKind::ALL).unwrap().expect("the thumb job comes first (priority 20)");
    let second = q.claim_next(&JobKind::ALL).unwrap().expect("a plugin job");
    let third = q.claim_next(&JobKind::ALL).unwrap();

    // Assert: 동시성 1 — 두 번째 plugin 행은 첫 plugin 작업이 끝나기 전까지 나오지 않는다
    assert_eq!((first.kind, first.asset_id), (JobKind::Thumb, Some(5)));
    assert_eq!(second.kind, JobKind::Plugin);
    assert!(third.is_none(), "plugin concurrency is 1");
    let counts = q.counts().unwrap();
    assert_eq!(counts.len(), JobKind::COUNT);
    let (_, plugin) = counts.iter().find(|(k, _)| *k == JobKind::Plugin).unwrap();
    assert_eq!((plugin.pending, plugin.running, plugin.failed), (1, 1, 1));
    // 수동 재시도(전체)가 plugin 의 failed 도 대기로 돌린다
    assert!(q.retry_failed(Some(JobKind::Plugin)).is_ok());
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'plugin' AND status = 'failed'"), 0);
}

#[test]
fn t_pjob_26_asset_purge_removes_plugin_kind_jobs_of_the_asset() {
    // Arrange
    let env = env_with_volume();
    let ids = insert_assets(&env, "vol-a", "lib", 2, "image");
    exec(
        &env,
        &format!(
            "INSERT INTO jobs (kind, asset_id, created_at, updated_at) VALUES ('plugin', {a}, 1, 1), ('plugin', {b}, 1, 1);",
            a = ids[0],
            b = ids[1]
        ),
    );

    // Act
    let target = ids[..1].to_vec();
    env.pool.write(Priority::High, move |c| Ok(mam_core::purge::purge_assets(c, 1, &target))).unwrap().unwrap();

    // Assert: 대상 자산의 plugin 작업만 사라진다
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM jobs WHERE kind = 'plugin' AND asset_id = {}", ids[0])), 0);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM jobs WHERE kind = 'plugin' AND asset_id = {}", ids[1])), 1);
}
