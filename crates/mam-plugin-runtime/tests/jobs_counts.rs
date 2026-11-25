//! P5 실패 재시도·작업 건수 (T-pjob-20 / 21) — `PluginJobs::retry_failed`·`task_counts` (명령 `plugin_retry_failed`·`plugin_task_counts` 의 본체).
//!
//! 분류: Happy(실패 -> 대기 -> 완료, 플러그인별 건수) / Boundary(실패 0건·작업 없는 플러그인) / Validation(설치되지 않은 플러그인) /
//! Business rule(다른 플러그인 행·done 행은 건드리지 않음, 반환값 = 되돌린 **자산 수**(중복 제거) — 작업 수가 아니다).
//! 실제 SQLite·실제 워커. EXPLAIN QUERY PLAN 은 `mam-db` 의 `plugin_tasks_eqp.rs` (T-pjob-21).

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_plugin_runtime::jobs::PluginJobsError;
use mam_plugin_runtime::{AssetKind, PluginTaskCount, ProviderKind};
use serde_json::json;

const P: &str = "com.example.script";
const Q: &str = "com.example.follower";

/// provider 두 개 -> 자산 하나에 작업 두 개
fn scripted() -> Plugin {
    Plugin::new(P, "scripted")
        .provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
        .provider("meta.b", ProviderKind::Metadata, &[AssetKind::Image])
}

fn count(plugin: &str, pending: u64, failed: u64) -> PluginTaskCount {
    PluginTaskCount {
        plugin_id: plugin.to_owned(),
        pending,
        failed,
    }
}

/// P 의 모든 작업(자산 3 x provider 2 = 6)을 스크립트 오류로 `failed` 로 만든다
fn failed_world() -> (Env, Vec<i64>) {
    let env = Env::new();
    env.install(scripted());
    let assets: Vec<i64> = (0..3)
        .map(|i| env.add_asset("image", &format!("{i}.jpg")))
        .collect();
    env.invoker
        .script((0..6).map(|_| json!({"throw": "boom"})).collect());
    assets.iter().for_each(|a| {
        env.edit(*a);
    });
    env.jobs.feed_tick();
    env.run(3);
    (env, assets)
}

#[test]
fn t_pjob_20_retry_failed_returns_failed_tasks_to_pending_and_requeues_one_job_per_asset() {
    // Arrange: 6 개 모두 failed + 다른 플러그인의 failed 행·P 의 done 행
    let (env, assets) = failed_world();
    assert_eq!(env.jobs.task_counts().unwrap(), vec![count(P, 0, 6)]);
    assert_eq!(
        env.task(P, "meta.run", assets[0])
            .and_then(|t| t.last_error)
            .as_deref(),
        Some("script_error")
    );
    env.install(Plugin::new(Q, "follower").provider(
        "meta.follow",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    env.seed_task(
        Q,
        "meta.follow",
        assets[0],
        2,
        "failed",
        Some("script_error"),
    );
    assert!(env.jobs_rows().is_empty());

    // Act
    let returned = env.jobs.retry_failed(P).unwrap();

    // Assert: 반환값은 되돌린 자산 수(3) — 작업 수(6)가 아니다. 시도·오류 초기화, 자산당 jobs 한 행
    assert_eq!(returned, 3);
    let t = env.task(P, "meta.b", assets[2]).unwrap();
    assert_eq!(
        (t.status.as_str(), t.attempts, t.last_error),
        ("pending", 0, None)
    );
    assert_eq!(
        env.jobs_rows()
            .iter()
            .map(|j| (j.asset, j.status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (assets[0], "pending"),
            (assets[1], "pending"),
            (assets[2], "pending")
        ]
    );
    assert_eq!(
        env.jobs.task_counts().unwrap(),
        vec![count(Q, 0, 1), count(P, 6, 0)],
        "the other plugin's failed row is untouched"
    );

    // Act 2: 이제 스크립트 오류 없이 실행 -> 모두 완료
    env.run(3);
    assert_eq!((env.pending_tasks(P), env.count(&format!("SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{P}' AND status = 'done'"))), (0, 6));
    assert_eq!(
        env.jobs.retry_failed(P).unwrap(),
        0,
        "nothing failed -> nothing to retry"
    );
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_20_retry_failed_for_a_plugin_that_is_not_installed_changes_nothing() {
    // Arrange: 실패 행이 있는 플러그인의 설치 표시를 지운다 (제거 중)
    let (env, _) = failed_world();
    env.delete_plugin_settings(P);

    // Act / Assert
    assert!(matches!(
        env.jobs.retry_failed(P),
        Err(PluginJobsError::NotInstalled)
    ));
    assert!(matches!(
        env.jobs.retry_failed("com.example.ghost"),
        Err(PluginJobsError::NotInstalled)
    ));
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{P}' AND status = 'failed'")), 6);
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_21_counts_are_per_installed_plugin_ordered_by_id_and_include_plugins_without_tasks() {
    // Arrange: P pending 2·failed 1·done 1, Q pending 1, R 작업 없음, 제거된 G 의 남은 행(keepData)
    let env = Env::new();
    env.install(scripted());
    env.install(Plugin::new(Q, "follower").provider(
        "meta.follow",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    env.install(Plugin::new("com.example.idle", "follower"));
    let a = env.bulk_assets(5, "image");
    for (plugin, asset, status) in [
        (P, 0, "pending"),
        (P, 1, "pending"),
        (P, 2, "failed"),
        (P, 3, "done"),
        (Q, 0, "pending"),
        ("com.example.gone", 0, "pending"),
    ] {
        env.seed_task(plugin, "x", a[asset], 1, status, None);
    }

    // Act
    let counts = env.jobs.task_counts().unwrap();

    // Assert: 플러그인 ID 순, 작업 없는 설치 플러그인은 0/0, 설치되지 않은 플러그인은 목록에 없다
    assert_eq!(
        counts,
        vec![
            count(Q, 1, 0),
            count("com.example.idle", 0, 0),
            count(P, 2, 1)
        ]
    );

    // Act 2: P 의 대기 작업 하나가 끝나면 건수가 바뀐다
    env.exec(&format!("UPDATE plugin_tasks SET status = 'done', done_revision = 1 WHERE plugin_ref = (SELECT plugin_ref FROM plugins WHERE plugin_id = '{P}') AND asset_id = {}", a[0]));

    // Assert 2
    assert_eq!(env.jobs.task_counts().unwrap()[2], count(P, 1, 1));
}
