//! P5 상태 변화에 따른 작업 처리 (T-pjob-15 / 16 / 17) — 비활성 플러그인·비활성 자산·사라진 provider.
//!
//! 분류: Business rule. 꺼진/제거된 플러그인의 작업은 지우지 않고 대기로 남기고(켜면 되살림), 비활성 자산은 호출 없이 닫고,
//! 업그레이드로 사라진 provider 의 행은 지운다. 실제 SQLite·실제 워커(provider 가 실제로 불리는 경로만).

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_core::jobs::JobOutcome;
use mam_plugin_runtime::{AssetKind, JobHooks, ProviderKind};

const P: &str = "com.example.state";

fn writer() -> Plugin {
    Plugin::new(P, "ok_writer").provider("meta.extract", ProviderKind::Metadata, &[AssetKind::Image])
}

fn queued_asset(env: &Env) -> i64 {
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    asset
}

#[test]
fn t_pjob_15_disabled_plugin_keeps_its_task_pending_and_the_enabled_hook_requeues_it() {
    // Arrange: 작업이 올라간 뒤 사용자가 플러그인을 끈다
    let env = Env::new();
    env.install(writer());
    let asset = queued_asset(&env);
    env.disable(P);

    // Act 1
    let done = env.run(1);

    // Assert 1: 호출 없이 작업은 끝나고(행 삭제) provider 작업은 대기로 남는다 — 시도·실패 없음
    assert!(matches!(done[0].outcome, Ok(JobOutcome::Done)));
    assert_eq!(env.invoker.call_count(), 0);
    let t = env.task(P, "meta.extract", asset).unwrap();
    assert_eq!((t.status.as_str(), t.attempts, t.last_error), ("pending", 0, None));
    assert!(env.jobs_rows().is_empty());

    // Act 2: 다시 켠다 -> `enabled` 훅이 대기 작업을 `jobs` 에 다시 올린다
    env.enable(P);
    let requeued = env.jobs_rows().len();
    env.run(1);

    // Assert 2
    assert_eq!(requeued, 1);
    assert_eq!(env.task(P, "meta.extract", asset).unwrap().status, "done");
    assert_eq!(env.kv(P, asset, "kind").as_deref(), Some("\"metadata\""));
}

#[test]
fn t_pjob_15_disabled_between_lookup_and_invoke_and_an_uninstalled_plugin_also_stay_pending() {
    // Arrange: 첫 호출 직전에 (자동 비활성처럼) 꺼진다
    let env = Env::new();
    env.install(writer());
    let first = queued_asset(&env);
    let invoker = env.invoker.clone();
    env.invoker.set_before(move |_| invoker.set_enabled(P, false));

    // Act 1
    env.run(1);

    // Assert 1: `RunError::Disabled` -> 대기 유지, 시도·실패 없음
    let t = env.task(P, "meta.extract", first).unwrap();
    assert_eq!((t.status.as_str(), t.attempts, t.last_error), ("pending", 0, None));
    assert_eq!(env.kv_rows(), 0);

    // Act 2: 런타임에서 항목이 사라진(`NotInstalled`) 플러그인
    env.invoker.clear_hooks();
    env.invoker.set_enabled(P, true);
    env.invoker.set_installed(P, false);
    env.jobs.enabled(&common_jobs::plugin_id(P));
    env.run(1);

    // Assert 2
    let t = env.task(P, "meta.extract", first).unwrap();
    assert_eq!((t.status.as_str(), t.attempts), ("pending", 0));
    assert_eq!(env.invoker.call_count(), 1, "only the first (disabled-at-invoke) call was made");
}

#[test]
fn t_pjob_16_inactive_assets_are_closed_without_invoking_and_the_task_is_done_inactive() {
    // Arrange: 병합(superseded)·삭제(deleted)된 자산의 작업이 올라가 있다
    let env = Env::new();
    env.install(writer());
    let merged = env.add_asset("image", "m.jpg");
    let deleted = env.add_asset("image", "d.jpg");
    for a in [merged, deleted] {
        env.edit(a);
    }
    env.jobs.feed_tick();
    env.set_status(merged, "superseded");
    env.set_status(deleted, "deleted");

    // Act
    let done = env.run(2);

    // Assert: 호출 없음, done(inactive) 로 닫힘 — 다시 대기하지 않는다
    assert!(done.iter().all(|d| matches!(d.outcome, Ok(JobOutcome::Done))));
    assert_eq!(env.invoker.call_count(), 0);
    for a in [merged, deleted] {
        let t = env.task(P, "meta.extract", a).unwrap();
        assert_eq!((t.status.as_str(), t.last_error.as_deref(), t.done), ("done", Some("inactive"), t.target));
    }
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_16_asset_turning_inactive_during_the_invoke_writes_nothing_and_closes_the_task() {
    // Arrange: provider 가 도는 사이 자산이 삭제된다
    let env = Env::new();
    env.install(writer());
    let asset = queued_asset(&env);
    let pool = env.pool.clone();
    env.invoker.set_after(move |c| {
        let id = c.asset;
        pool.write(mam_db::Priority::High, move |conn| Ok(conn.execute("UPDATE media_assets SET status = 'deleted' WHERE asset_id = ?1", [id])?)).unwrap();
    });

    // Act
    let done = env.run(1);

    // Assert: 기록은 `AssetInactive` 로 막혀 KV·태그가 없고 작업은 done(inactive)
    assert!(matches!(done[0].outcome, Ok(JobOutcome::Done)));
    assert_eq!((env.kv_rows(), env.plugin_tags(asset)), (0, Vec::<String>::new()));
    let t = env.task(P, "meta.extract", asset).unwrap();
    assert_eq!((t.status.as_str(), t.last_error.as_deref()), ("done", Some("inactive")));
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_17_task_rows_of_unknown_or_export_providers_are_deleted_and_known_ones_still_run() {
    // Arrange: 업그레이드로 provider 가 사라졌다(`gone.provider`), 내보내기로 바뀐 이름(`exp.csv`) + 정상 `meta.extract`
    let env = Env::new();
    env.install(writer().provider("exp.csv", ProviderKind::Export, &[AssetKind::Image]));
    let asset = queued_asset(&env);
    env.seed_task(P, "gone.provider", asset, 2, "pending", None);
    env.seed_task(P, "exp.csv", asset, 2, "pending", None);

    // Act
    env.run(1);

    // Assert: 알 수 없는 provider·내보내기 provider 행은 삭제, 정상 provider 는 실행
    assert!(env.task(P, "gone.provider", asset).is_none());
    assert!(env.task(P, "exp.csv", asset).is_none());
    assert_eq!(env.task(P, "meta.extract", asset).unwrap().status, "done");
    assert_eq!(env.invoker.call_count(), 1);
}
