//! P5 + P6 통합 (명세 §D) — 실제 `PluginRuntime`(실제 워커·설치·제거·활성)에 `PluginJobs` 를 `src-tauri` 와 같은 방식으로 연결한다.
//!
//! 분류: Happy(사용자 태그 편집 -> outbox -> 피드 -> 실제 JobRunner -> 실제 워커 provider -> KV·태그 기록 -> 완료) /
//! Concurrency(제거 중 진행 중 호출·피드·전체 실행·계속되는 편집 — 제거 뒤 어떤 쓰기도 남지 않는다, 워커 프로세스 종료를 PID 로 확인) /
//! Business rule(비활성은 공급·실행하지 않고, 활성화 훅이 대기 작업을 다시 올린다).
//! 실제 SQLite·실제 워커. 대기는 채널·큐 condvar·join 뿐이다.

mod common_integ;
mod common_rt;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use common_integ::{ID, Integ};
use common_rt::pid_alive;
use mam_core::jobs::{JobKind, JobOutcome};
use mam_plugin_host::{HostState, InvokeKind, InvokeScope};
use mam_plugin_runtime::jobs::JobsTuning;
use serde_json::Value;

fn pending_jobs(it: &Integ) -> i64 {
    it.count("SELECT COUNT(*) FROM jobs WHERE kind = 'plugin' AND status = 'pending'")
}

fn rows_of(it: &Integ, table: &str) -> i64 {
    it.count(&format!(
        "SELECT COUNT(*) FROM {table} JOIN plugins USING (plugin_ref) WHERE plugin_id = '{ID}'"
    ))
}

#[test]
fn t_pjob_int_01_user_tag_edit_flows_through_real_feed_runner_and_worker_to_kv_tag_and_done() {
    // Arrange
    let it = Integ::new();
    it.install();
    let asset = it.rt.add_asset("photo.jpg");
    assert_eq!(
        it.cursor(),
        Some(0),
        "the install hook put the cursor at the end of the outbox"
    );

    // Act: 사용자 편집 -> 피드 한 번 -> 실제 러너가 실행기를 돌린다
    it.edit(asset, "user-tag");
    let more = it.jobs.feed_tick();
    let queued = (it.task(asset), pending_jobs(&it));
    let done = it.run(1);

    // Assert
    assert!(!more);
    assert_eq!(queued, (Some(("pending".to_owned(), 2, 0)), 1));
    assert!(matches!(done[0], Ok(JobOutcome::Done)));
    assert_eq!(it.kv(asset, "fileName").as_deref(), Some("\"photo.jpg\""));
    assert_eq!(it.kv(asset, "revision").as_deref(), Some("2"));
    assert_eq!(
        it.tags(asset),
        vec!["auto".to_owned(), "user-tag".to_owned()]
    );
    assert_eq!(
        it.task(asset),
        Some(("done".to_owned(), 2, 3)),
        "own tag event (revision 3) is absorbed by the loop guard"
    );
    assert!(!it.jobs.feed_tick() && pending_jobs(&it) == 0);
    let host = it.rt.rt.host_for_test(ID).unwrap();
    assert!(
        matches!(host.state(), HostState::Running { .. }) && host.starts() == 1,
        "one real worker served the provider"
    );
}

#[test]
fn t_pjob_int_02_uninstall_during_an_in_flight_provider_with_feed_and_run_all_active_leaves_nothing_behind()
 {
    // Arrange: 느린 자산(provider 가 800 ms 돈다) 하나만 먼저 피드해 그 호출이 진행 중이 되게 한다
    let it = Integ::with_spy();
    it.install();
    it.jobs.set_tuning(JobsTuning {
        feed_idle: Duration::from_millis(5),
        feed_busy_pause: Duration::from_millis(5),
        run_all_batch: 2,
        ..JobsTuning::default()
    });
    let slow = it.rt.add_asset("slow1.jpg");
    let others: Vec<i64> = (0..30)
        .map(|i| it.rt.add_asset(&format!("n{i}.jpg")))
        .collect();
    let host = it.rt.rt.host_for_test(ID).unwrap();
    host.invoke(
        InvokeKind::Provider,
        "ping",
        Value::Null,
        InvokeScope::empty(),
    )
    .expect("warm-up");
    let HostState::Running { pid } = host.state() else {
        panic!("worker should be running after the warm-up")
    };
    it.edit(slow, "u0");
    it.jobs.feed_tick();
    it.start_runner();
    it.started
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(60))
        .expect("the slow provider call started");

    // Act: 호출이 진행 중인 동안 피드 스레드·전체 실행·계속되는 편집을 켜고 제거한다
    others
        .iter()
        .enumerate()
        .for_each(|(i, a)| it.edit(*a, &format!("u{}", i + 1)));
    it.jobs.start_feed();
    assert!(it.jobs.run_all(ID, "meta.extract").unwrap().started);
    let (stop, edits) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let editor = {
        let (it_edit, stop, edits, target) = (
            it.editor(),
            Arc::clone(&stop),
            Arc::clone(&edits),
            others[0],
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                it_edit(
                    target,
                    &format!("race-{}", edits.fetch_add(1, Ordering::SeqCst)),
                );
                std::thread::yield_now();
            }
        })
    };
    it.rt
        .rt
        .uninstall(ID, false)
        .expect("uninstall waits for the in-flight call and succeeds");
    stop.store(true, Ordering::SeqCst);
    editor.join().unwrap();
    it.stop_runner();
    it.jobs.stop();

    // Assert: 제거 뒤 쓰기가 거부돼 KV·작업·설정(커서 포함) 어느 것도 남지 않고, 워커 프로세스가 사라졌다
    assert_eq!(
        (rows_of(&it, "plugin_kv"), rows_of(&it, "plugin_tasks")),
        (0, 0)
    );
    assert_eq!(
        it.count(&format!(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin\\_%:{ID}' ESCAPE '\\'"
        )),
        0
    );
    assert_eq!(it.cursor(), None);
    assert!(
        edits.load(Ordering::SeqCst) > 0,
        "edits kept producing outbox events"
    );
    assert!(
        !pid_alive(pid),
        "the worker process of the uninstalled plugin is gone"
    );
    assert!(!it.rt.plugins_dir.join(ID).exists());
}

#[test]
fn t_pjob_int_03_disabled_plugin_is_not_fed_or_run_and_enabling_requeues_its_pending_work() {
    // Arrange: 대기 작업과 jobs 행이 있는 상태에서 끈다
    let it = Integ::new();
    it.install();
    let asset = it.rt.add_asset("photo.jpg");
    it.edit(asset, "t1");
    it.jobs.feed_tick();
    assert_eq!(
        (it.task(asset), pending_jobs(&it)),
        (Some(("pending".to_owned(), 2, 0)), 1)
    );
    it.rt.rt.set_enabled(ID, false, "en").unwrap();
    let cursor = it.cursor();

    // Act 1: 꺼진 동안 작업이 실행돼도 아무것도 하지 않고(대기 유지), 새 편집은 공급되지 않는다
    let done = it.run(1);
    it.edit(asset, "t2");
    it.jobs.feed_tick();

    // Assert 1
    assert!(matches!(done[0], Ok(JobOutcome::Done)));
    assert_eq!(
        (it.kv(asset, "fileName"), it.task(asset)),
        (None, Some(("pending".to_owned(), 2, 0))),
        "stays pending, not failed"
    );
    assert_eq!(
        it.cursor(),
        cursor,
        "the feed cursor stops while the plugin is disabled"
    );
    assert_eq!(pending_jobs(&it), 0);

    // Act 2: 켜면 훅이 대기 작업을 jobs 에 다시 올린다. 러너를 멈춰 둔 채(큐 일시정지) 올라간 행을 확인한 뒤 재개한다
    it.queue.pause(Some(JobKind::Plugin)).unwrap();
    it.rt.rt.set_enabled(ID, true, "en").unwrap();
    let requeued = pending_jobs(&it);
    it.queue.resume(Some(JobKind::Plugin)).unwrap();
    it.jobs.feed_tick();
    it.wait_until("task done at revision 3", |i| {
        i.task(asset).is_some_and(|t| t.0 == "done" && t.1 == 3)
    });
    it.settle();

    // Assert 2: 훅이 올린 행이 있었고, 피드가 꺼진 동안의 편집(revision 3)까지 따라잡아 처리했다 (러너와의 경합 순서는 가리지 않는다)
    assert_eq!(requeued, 1);
    assert_eq!(it.kv(asset, "revision").as_deref(), Some("3"));
    assert_eq!(
        it.tags(asset),
        vec!["auto".to_owned(), "t1".to_owned(), "t2".to_owned()]
    );
    assert_eq!(pending_jobs(&it), 0);
}
