//! P5 피드의 스캔 양보 (T-pjob-28) — 스캔 계열(scan/meta/thumb/hash) 작업이 대기·실행 중이면 피드는 커서를 올리지도 쓰지도 않는다.
//!
//! 분류: Happy(스캔 끝나면 재개) / Boundary(대기·실행 두 상태, 계열 밖 종류·일시정지된 종류는 막지 않음, 배치 500) / Business rule(이미 올라간 plugin 작업은 계속).
//! `feed_tick()` 으로 결정적으로 돈다 — 시계·sleep 없음. 실제 SQLite.

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_core::jobs::JobKind;
use mam_plugin_runtime::jobs::FEED_BATCH;
use mam_plugin_runtime::{AssetKind, ProviderKind};

const P: &str = "com.example.scanyield";

fn world() -> (Env, i64) {
    let env = Env::new();
    env.install(Plugin::new(P, "scripted").provider(
        "meta.run",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    let asset = env.add_asset("image", "a.jpg");
    (env, asset)
}

fn scan_job(env: &Env, asset: i64, kind: &str, status: &str) {
    env.exec(&format!("INSERT INTO jobs (kind, asset_id, priority, status, created_at, updated_at) VALUES ('{kind}', {asset}, 50, '{status}', 0, 0);"));
}

#[test]
fn t_pjob_28_feed_waits_while_a_scan_family_job_is_pending_or_running_and_resumes_after() {
    for (kind, status) in [
        ("scan", "pending"),
        ("meta", "running"),
        ("thumb", "pending"),
        ("hash", "running"),
    ] {
        // Arrange
        let (env, asset) = world();
        let cursor = env.cursor(P);
        env.edit(asset);
        scan_job(&env, asset, kind, status);

        // Act: 스캔 작업이 있는 동안
        let more = env.jobs.feed_tick();

        // Assert: 커서·작업 모두 그대로
        assert!(!more, "{kind}/{status}");
        assert_eq!(
            env.cursor(P),
            cursor,
            "{kind}/{status}: cursor must not move"
        );
        assert_eq!(env.task_rows(P), 0, "{kind}/{status}: no write tx");

        // Act: 스캔 작업이 끝나면
        env.exec(&format!("DELETE FROM jobs WHERE kind = '{kind}'"));
        env.jobs.feed_tick();

        // Assert: 재개되어 끝까지 간다
        assert_eq!(env.cursor(P), Some(env.max_event()), "{kind}/{status}");
        assert_eq!(env.task_rows(P), 1);
    }
}

#[test]
fn t_pjob_28_other_kinds_and_paused_scan_kinds_do_not_hold_the_feed() {
    // Arrange: ai(장시간 사용자 작업)·index 는 계열 밖, meta 는 일시정지 상태라 진행하지 않는다
    let (env, asset) = world();
    env.edit(asset);
    scan_job(&env, asset, "ai", "running");
    scan_job(&env, asset, "index", "pending");
    scan_job(&env, asset, "meta", "pending");
    env.queue.pause(Some(JobKind::Meta)).unwrap();

    // Act
    env.jobs.feed_tick();

    // Assert
    assert_eq!(env.cursor(P), Some(env.max_event()));
    assert_eq!(env.task_rows(P), 1);
}

#[test]
fn t_pjob_28_plugin_jobs_already_queued_keep_running_while_the_scan_is_busy() {
    // Arrange: 스캔이 없을 때 작업 1개를 올려 둔다
    let (env, asset) = world();
    env.edit(asset);
    env.jobs.feed_tick();
    assert_eq!(env.jobs_rows().len(), 1);
    scan_job(&env, asset + 1_000, "thumb", "running");

    // Act: 스캔이 도는 중에도 plugin 작업은 실행된다
    env.run(1);

    // Assert
    assert_eq!(env.pending_tasks(P), 0);
}

#[test]
fn t_pjob_28_one_tick_folds_at_most_500_event_ids() {
    // Arrange
    assert_eq!(FEED_BATCH, 500);
    let (env, asset) = world();
    let start = env.cursor(P).unwrap();
    for _ in 0..1_200 {
        env.edit(asset);
    }

    // Act
    let more = env.jobs.feed_tick();

    // Assert: 한 번에 500, 남았으면 `true`
    assert!(more);
    assert_eq!(env.cursor(P), Some(start + 500));
}
