//! P5 변경 피드 (T-pjob-02 / 05 / 06 / 07 / 09) — `search_outbox` -> `plugin_tasks` + `jobs`.
//!
//! 분류: Concurrency(02 멱등 재공급) / Validation(05 비활성·삭제 자산) / Business rule(06 종류 필터·내보내기 제외, 09 커서 초기화) /
//! Boundary(07 backpressure). 피드는 `feed_tick()` 으로 스레드 없이 결정적으로 돈다. 실제 SQLite, provider 는 호출되지 않는 시험이 많다.

mod common_jobs;

use common_jobs::{Env, Plugin};
use mam_plugin_runtime::jobs::JobsTuning;
use mam_plugin_runtime::{AssetKind, JobHooks, ProviderKind};

const P: &str = "com.example.feed";

fn scripted() -> Plugin {
    Plugin::new(P, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
}

#[test]
fn t_pjob_02_rewinding_the_cursor_and_refeeding_makes_no_duplicates_and_no_extra_invokes() {
    // Arrange: 한 번 완전히 처리한 자산 (사용자 편집 + 플러그인 자기 태그 이벤트가 outbox 에 있다)
    let env = Env::new();
    env.install(Plugin::new(P, "ok_writer").provider("meta.extract", ProviderKind::Metadata, &[AssetKind::Image]));
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    env.run(1);
    let (rows, calls, before) = (env.task_rows(P), env.invoker.call_count(), env.task(P, "meta.extract", asset).unwrap());

    // Act: 커서를 처음으로 되감고 같은 구간을 다시 공급
    env.set_cursor(P, 0);
    let more = env.jobs.feed_tick();

    // Assert: 행은 그대로, 완료 행은 되살아나지 않고, 새 작업도 호출도 없다
    assert!(!more);
    assert_eq!(env.cursor(P), Some(env.max_event()));
    assert_eq!((env.task_rows(P), env.invoker.call_count()), (rows, calls));
    let after = env.task(P, "meta.extract", asset).unwrap();
    assert_eq!((after.status.as_str(), after.done, after.attempts), (before.status.as_str(), before.done, before.attempts));
    assert_eq!(after.target, 3, "target only rises (the own tag event is folded in), never revives the row");
    assert!(env.jobs_rows().is_empty());
}

#[test]
fn t_pjob_05_events_of_inactive_or_vanished_assets_make_no_row_advance_the_cursor_and_do_not_break_the_tx() {
    // Arrange: 활성 1 + 병합(superseded) + 삭제(deleted) + 행이 사라진(hard delete) 자산의 이벤트
    let env = Env::new();
    env.install(scripted());
    let [active, merged, deleted, gone] = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"].map(|n| env.add_asset("image", n));
    for a in [active, merged, deleted, gone] {
        env.edit(a);
    }
    env.set_status(merged, "superseded");
    env.set_status(deleted, "deleted");
    env.exec(&format!("DELETE FROM file_locations WHERE asset_id = {gone}; DELETE FROM media_assets WHERE asset_id = {gone};"));

    // Act
    let more = env.jobs.feed_tick();

    // Assert: 활성 자산만 작업이 생기고 커서는 끝까지 간다
    assert!(!more);
    assert_eq!(env.cursor(P), Some(env.max_event()));
    assert_eq!(env.task_rows(P), 1);
    assert!(env.task(P, "meta.run", active).is_some());
    assert!([merged, deleted, gone].iter().all(|a| env.task(P, "meta.run", *a).is_none()));
    assert_eq!(env.jobs_rows().len(), 1);
}

#[test]
fn t_pjob_06_asset_kinds_filter_the_feed_and_export_providers_are_never_fed() {
    // Arrange: image 전용 metadata, 전체 analysis, 그리고 (잘못된 호출자가 auto 목록에 섞은) export provider
    let env = Env::new();
    env.install(
        Plugin::new(P, "scripted")
            .provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
            .provider("ana.c", ProviderKind::Analysis, &[])
            .provider("exp.csv", ProviderKind::Export, &[]),
    );
    env.invoker.leak_export_into_auto(true);
    let image = env.add_asset("image", "a.jpg");
    let video = env.add_asset("video", "b.mp4");
    let audio = env.add_asset("audio", "c.wav");
    for a in [image, video, audio] {
        env.edit(a);
    }

    // Act
    env.jobs.feed_tick();

    // Assert
    let has = |provider: &str, asset: i64| env.task(P, provider, asset).is_some();
    assert!(has("meta.run", image) && !has("meta.run", video) && !has("meta.run", audio), "image-only metadata");
    assert!(has("ana.c", image) && has("ana.c", video) && has("ana.c", audio), "empty assetKinds = all");
    assert_eq!(env.count("SELECT COUNT(*) FROM plugin_tasks JOIN plugin_providers USING (provider_ref) WHERE name = 'exp.csv'"), 0, "export providers are never fed");
    assert_eq!(env.task_rows(P), 4);
    assert_eq!(env.jobs_rows().len(), 3, "one job per asset");
}

#[test]
fn t_pjob_07_backpressure_stops_the_cursor_at_the_cap_then_resumes_after_draining() {
    // Arrange: 상한 3, 한 틱에 이벤트 2건. 자산 10개 = 이벤트 10건
    let env = Env::new();
    env.install(scripted());
    env.jobs.set_tuning(JobsTuning { pending_cap: 3, feed_batch: 2, ..JobsTuning::default() });
    let assets: Vec<i64> = (0..10).map(|i| env.add_asset("image", &format!("{i}.jpg"))).collect();
    for a in &assets {
        env.edit(*a);
    }

    // Act 1: 상한(3)을 넘는 순간(4건) 커서가 멈춘다
    let (t1, t2, t3) = (env.jobs.feed_tick(), env.jobs.feed_tick(), env.jobs.feed_tick());

    // Assert 1
    assert_eq!((t1, t2, t3), (true, true, false));
    assert_eq!((env.cursor(P), env.pending_tasks(P)), (Some(4), 4));
    assert!(!env.jobs.feed_tick(), "still stalled until the backlog drains");
    assert_eq!(env.cursor(P), Some(4));

    // Act 2: 배출 -> 재개를 모든 이벤트가 처리될 때까지 반복
    let mut rounds = 0;
    while env.cursor(P) != Some(env.max_event()) || env.pending_tasks(P) > 0 {
        rounds += 1;
        assert!(rounds < 40, "feed must make progress");
        env.run_pending_jobs();
        env.jobs.feed_tick();
    }

    // Assert 2: 모든 자산이 정확히 한 번 처리됨 (이벤트 누락·중복 없음)
    assert!(assets.iter().all(|a| env.task(P, "meta.run", *a).is_some_and(|t| t.status == "done" && t.done == 2)));
    assert_eq!(env.invoker.call_count(), 10);
}

#[test]
fn t_pjob_09_fresh_install_hook_puts_the_cursor_at_the_end_and_a_missing_cursor_is_initialized_lazily() {
    // Arrange: 설치 이전의 이벤트 3건
    let env = Env::new();
    let assets: Vec<i64> = (0..3).map(|i| env.add_asset("image", &format!("{i}.jpg"))).collect();
    for a in &assets {
        env.edit(*a);
    }

    // Act 1: 설치 훅(fresh=true) — 설치 이전 이벤트는 작업이 되지 않는다
    env.install(scripted());
    env.jobs.feed_tick();

    // Assert 1
    assert_eq!((env.cursor(P), env.task_rows(P)), (Some(3), 0));

    // Act 2: 이후 편집은 공급된다. 이미 있는 커서는 훅이 다시 불려도(fresh) 되돌리지 않고, fresh=false 는 건드리지 않는다
    env.edit(assets[0]);
    env.jobs.installed(&common_jobs::plugin_id(P), true);
    env.jobs.installed(&common_jobs::plugin_id(P), false);
    let kept = env.cursor(P);
    env.jobs.feed_tick();

    // Assert 2
    assert_eq!(kept, Some(3));
    assert_eq!(env.task_rows(P), 1);

    // Act 3: 훅이 없었던 플러그인(커서 없음) — 첫 틱이 현재 끝으로 초기화하고 그 이전 이벤트는 건너뛴다
    let lazy = "com.example.lazy";
    env.install(common_jobs::without_hook(Plugin::new(lazy, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])));
    assert_eq!(env.cursor(lazy), None);
    env.jobs.feed_tick();
    let initialized = (env.cursor(lazy), env.task_rows(lazy));
    env.edit(assets[1]);
    env.jobs.feed_tick();

    // Assert 3
    assert_eq!(initialized, (Some(env.max_event() - 1), 0));
    assert_eq!(env.task_rows(lazy), 1, "events after the lazy init are fed");
}
