//! P5 실행기 정상 경로 (T-pjob-01 / 03 / 04) — tag 편집 -> outbox -> 피드 -> 작업 -> 실제 워커 provider -> KV·태그 기록 -> 완료.
//!
//! 분류: Happy(01) / Business rule(03 revision 역전·늦은 완료, 04 루프 방지) / Regression(03: 실행 중 편집이 삼켜지지 않는다).
//! 실제 SQLite·실제 워커·실제 `UserData` 편집. 대기는 러너 채널/큐 condvar 뿐이다.

mod common_jobs;

use common_jobs::{Env, Plugin, snapshot};
use mam_core::jobs::JobOutcome;
use mam_plugin_runtime::{AssetKind, ProviderKind};
use serde_json::json;

const WRITER: &str = "com.example.writer";
const FOLLOWER: &str = "com.example.follower";

fn writer() -> Plugin {
    Plugin::new(WRITER, "ok_writer").provider(
        "meta.extract",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    )
}

#[test]
fn t_pjob_01_tag_edit_flows_to_provider_then_kv_tag_and_done_and_originals_are_untouched() {
    // Arrange
    let env = Env::new();
    env.install(writer());
    let asset = env.add_asset("image", "photo.jpg");
    let before = snapshot(&env.lib);
    assert_eq!(
        env.cursor(WRITER),
        Some(0),
        "fresh install puts the cursor at the end of the outbox"
    );

    // Act: 사용자 편집 -> 피드 -> 실행
    let rev = env.edit(asset);
    let more = env.jobs.feed_tick();
    let queued = (
        env.task(WRITER, "meta.extract", asset)
            .map(|t| (t.status, t.target)),
        env.jobs_rows().len(),
    );
    let done = env.run(1);

    // Assert: 작업·`jobs` 가 만들어졌고 provider 가 revision 과 종류를 받았다
    assert_eq!((rev, more), (2, false));
    assert_eq!(queued, (Some(("pending".to_owned(), 2)), 1));
    let calls = env.invoker.calls();
    assert_eq!(
        (
            calls.len(),
            calls[0].plugin.as_str(),
            calls[0].provider.as_str(),
            calls[0].revision
        ),
        (1, WRITER, "meta.extract", 2)
    );
    // KV + 태그가 기록되고 작업은 완료 — 자기 태그 이벤트(revision 3)까지 처리한 것으로 본다(루프 방지)
    assert_eq!(
        env.kv(WRITER, asset, "fileName").as_deref(),
        Some("\"photo.jpg\"")
    );
    assert_eq!(
        (
            env.kv(WRITER, asset, "revision").as_deref(),
            env.kv(WRITER, asset, "kind").as_deref()
        ),
        (Some("2"), Some("\"metadata\""))
    );
    assert_eq!(env.plugin_tags(asset), vec!["auto".to_owned()]);
    let task = env.task(WRITER, "meta.extract", asset).unwrap();
    assert_eq!(
        (
            task.status.as_str(),
            task.done,
            task.attempts,
            task.last_error
        ),
        ("done", 3, 0, None)
    );
    assert_eq!(env.revision(asset), 3);
    assert!(matches!(done[0].outcome, Ok(JobOutcome::Done)));
    assert!(env.jobs_rows().is_empty(), "the job row is gone");
    assert_eq!(env.invoker.host(WRITER).starts(), 1);
    // 원본 보호: 내용·수정 시각 불변
    assert_eq!(snapshot(&env.lib), before);
}

#[test]
fn t_pjob_03_edit_during_the_run_is_not_swallowed_and_a_late_completion_keeps_the_task_pending() {
    // Arrange: provider 가 revision 2 를 보고 실행 중일 때 사용자가 다시 편집한다 (revision 3, 피드가 target 3 으로 올림)
    let env = Env::new();
    env.install(Plugin::new("com.example.script", "scripted").provider(
        "meta.run",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    env.invoker.script(vec![
        json!({"out": {"tags": ["auto"]}}),
        json!({"out": {"tags": ["auto"]}}),
    ]);
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let (editor, jobs) = (env.editor(), env.jobs.clone());
    env.invoker.set_before(move |c| {
        if c.n == 1 {
            editor.edit(c.asset);
            jobs.feed_tick();
        }
    });

    // Act: 첫 실행은 revision 2 로 완료(target 3 이라 pending 유지) -> 쌍둥이 작업이 revision 3 으로 다시 실행
    let done = env.run(2);

    // Assert: 예전에는 첫 완료가 자기 태그 revision(4)까지 done 으로 삼켜 두 번째 실행이 없었다
    let revs: Vec<i64> = env.invoker.calls().iter().map(|c| c.revision).collect();
    assert_eq!(revs, vec![2, 3], "the unseen edit is processed");
    assert!(
        done.iter()
            .all(|d| matches!(d.outcome, Ok(JobOutcome::Done)))
    );
    let t = env.task("com.example.script", "meta.run", asset).unwrap();
    assert_eq!((t.status.as_str(), t.target, t.done), ("done", 3, 3));
    // 두 번째 완료는 자기 이벤트(revision 4)를 아직 보지 못했다 -> 피드 뒤 한 번 더(태그가 이미 있어 no-op) 돌고 안정된다
    env.jobs.feed_tick();
    env.run(1);
    let revs: Vec<i64> = env.invoker.calls().iter().map(|c| c.revision).collect();
    assert_eq!(revs, vec![2, 3, 4]);
    assert_eq!(env.revision(asset), 4, "the extra run adds nothing");
    assert!(
        !env.jobs.feed_tick()
            && env.pending_tasks("com.example.script") == 0
            && env.jobs_rows().is_empty()
    );
}

#[test]
fn t_pjob_04_own_tag_event_does_not_repend_this_plugin_but_a_second_plugin_gets_a_task() {
    // Arrange: 같은 자산에 두 플러그인. 작성자는 태그를 붙이고, 팔로워는 KV 만 쓴다
    let env = Env::new();
    env.install(writer());
    env.install(Plugin::new(FOLLOWER, "follower").provider(
        "meta.follow",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    assert_eq!(
        env.jobs_rows().len(),
        1,
        "one job per asset, both plugins' tasks inside"
    );

    // Act: 한 작업이 두 플러그인을 처리 -> 작성자의 태그 이벤트(revision 3)를 피드 -> 팔로워만 다시 대기
    env.run(1);
    env.jobs.feed_tick();
    let after_feed = (
        env.task(WRITER, "meta.extract", asset).unwrap().status,
        env.task(FOLLOWER, "meta.follow", asset).unwrap(),
    );
    env.run(1);

    // Assert
    assert_eq!(
        after_feed.0, "done",
        "the writer's own event does not re-pend the writer"
    );
    assert_eq!(
        (
            after_feed.1.status.as_str(),
            after_feed.1.target,
            after_feed.1.done
        ),
        ("pending", 3, 2)
    );
    let calls: Vec<(String, i64)> = env
        .invoker
        .calls()
        .iter()
        .map(|c| (c.plugin.clone(), c.revision))
        .collect();
    // 같은 자산의 작업은 task_id 순 — 피드가 플러그인 ID 순으로 넣었으므로 follower 가 먼저다
    assert_eq!(
        calls,
        vec![
            (FOLLOWER.to_owned(), 2),
            (WRITER.to_owned(), 2),
            (FOLLOWER.to_owned(), 3)
        ]
    );
    assert_eq!(env.kv(FOLLOWER, asset, "seen").as_deref(), Some("3"));
    assert!(!env.jobs.feed_tick(), "stable: no further work");
    assert_eq!(env.pending_tasks(WRITER) + env.pending_tasks(FOLLOWER), 0);
}
