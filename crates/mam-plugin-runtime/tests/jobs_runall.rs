//! P5 전체 실행 (T-pjob-19) — 2,500 자산을 배치 1,000 으로 3 배치에 등록한다.
//!
//! 분류: Happy(전부 등록·실행) / Boundary(배치 경계·PENDING_CAP 무시·kind 필터) / Concurrency(single-flight, 정지, 배치 사이 제거) /
//! Business rule(최신 revision 건너뜀·내보내기 거부).
//! 실제 SQLite·실제 워커. 대기는 큐 condvar 뿐이다. 자산은 DB 에만 만든다(파일 없음 — 원본 보호는 T-pjob-01 이 본다).

mod common_jobs;

use std::time::Duration;

use common_jobs::{Env, Plugin, exporter, EXPORTER};
use mam_plugin_runtime::jobs::{JobsTuning, RunAllError};
use mam_plugin_runtime::{AssetKind, ProviderKind, RunError};

const P: &str = "com.example.script";
const BATCH: i64 = 1_000;

fn scripted() -> Plugin {
    Plugin::new(P, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
}

/// 배치 1,000·배치 사이 쉬지 않음·PENDING_CAP 3 (전체 실행은 상한을 무시해야 한다)
fn tune(env: &Env) {
    env.jobs.set_tuning(JobsTuning { run_all_batch: BATCH, feed_busy_pause: Duration::ZERO, pending_cap: 3, ..JobsTuning::default() });
}

/// 활성 이미지 2,500 + 비디오 50 (provider 는 이미지만)
fn world() -> (Env, Vec<i64>) {
    let env = Env::new();
    env.install(scripted());
    tune(&env);
    let images = env.bulk_assets(2_500, "image");
    env.bulk_assets(50, "video");
    (env, images)
}

fn started(env: &Env) -> bool {
    env.jobs.run_all(P, "meta.run").unwrap().started
}

#[test]
fn t_pjob_19_run_all_tasks_every_active_asset_in_three_batches_then_skips_up_to_date_work() {
    // Arrange
    let (mut env, images) = world();
    env.exec("CREATE TABLE t_touch (n INTEGER); CREATE TRIGGER t_touch_upd AFTER UPDATE ON plugin_tasks BEGIN INSERT INTO t_touch VALUES (1); END");

    // Act 1: 전체 실행 -> 3 배치 (1,000 / 1,000 / 550 중 이미지 500). 끝나면 재시작해 스레드를 join 한다
    assert!(started(&env));
    env.wait_until("2,500 plugin jobs", |e| e.plugin_jobs() == 2_500);
    env.restart();
    tune(&env);

    // Assert 1: 이미지만 모두 pending(revision 1), 비디오는 없음, PENDING_CAP(3) 은 적용되지 않았다
    assert_eq!((env.task_rows(P), env.pending_tasks(P), env.plugin_jobs()), (2_500, 2_500, 2_500));
    assert_eq!(env.task(P, "meta.run", images[0]).map(|t| (t.status, t.target, t.done)), Some(("pending".to_owned(), 1, 0)));
    assert!(env.task(P, "meta.run", images[2_499]).is_some());
    assert_eq!(env.count("SELECT COUNT(*) FROM plugin_tasks t JOIN media_assets a ON a.asset_id = t.asset_id WHERE a.media_type <> 'image'"), 0);

    // Act 2: 작업을 실제로 모두 실행한다 (실제 워커)
    env.run(2_500);
    let calls = env.invoker.call_count();

    // Assert 2
    assert_eq!((calls, env.pending_tasks(P), env.plugin_jobs()), (2_500, 0, 0));
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{P}' AND status = 'done' AND done_revision = 1")), 2_500);

    // Act 3: 다시 전체 실행 — 2,500 행을 모두 훑지만(UPDATE 트리거) 현재 revision 이 끝난 행은 되살리지 않는다
    env.exec("DELETE FROM t_touch");
    assert!(started(&env));
    env.wait_until("rerun scanned every row", |e| e.count("SELECT COUNT(*) FROM t_touch") == 2_500);

    // Assert 3: 새 대기 작업·jobs 행·호출 없음
    assert_eq!((env.pending_tasks(P), env.plugin_jobs(), env.invoker.call_count()), (0, 0, calls));

    // Act 4: 자산 하나만 편집(revision 2) 후 다시 전체 실행 -> 그 자산만 다시 대기
    env.restart();
    tune(&env);
    env.edit(images[7]);
    env.exec("DELETE FROM t_touch");
    assert!(started(&env));
    env.wait_until("rerun after edit", |e| e.plugin_jobs() == 1);
    env.run(1);

    // Assert 4
    let last = env.invoker.calls().pop().unwrap();
    assert_eq!((env.invoker.call_count(), last.asset, last.revision), (calls + 1, images[7], 2));
    assert_eq!(env.task(P, "meta.run", images[7]).map(|t| (t.status, t.done)), Some(("done".to_owned(), 2)));
}

#[test]
fn t_pjob_19_second_concurrent_call_is_not_started_and_stop_ends_the_run_without_more_rows() {
    // Arrange: 배치 사이를 길게 쉬게 해 첫 실행이 살아 있는 동안 두 번째 호출을 한다
    let (env, _) = world();
    env.jobs.set_tuning(JobsTuning { run_all_batch: BATCH, feed_busy_pause: Duration::from_secs(60), ..JobsTuning::default() });

    // Act
    let (first, second) = (started(&env), started(&env));
    env.wait_until("first batch", |e| e.plugin_jobs() == 1_000);
    env.jobs.stop();

    // Assert: 두 번째는 시작되지 않았고, stop 이 60 초 잠을 깨워 첫 배치 뒤에서 멈췄다
    assert_eq!((first, second), (true, false));
    assert_eq!((env.plugin_jobs(), env.task_rows(P)), (1_000, 1_000));
    assert!(!started(&env), "after stop(), a new run-all is refused");
    assert_eq!(env.plugin_jobs(), 1_000);
}

#[test]
fn t_pjob_19_run_all_stops_when_the_plugin_is_uninstalled_between_batches() {
    // Arrange: 첫 배치의 마지막 자산 jobs 행이 들어갈 때 같은 트랜잭션에서 설정 행이 지워진다 (= 제거가 배치 1 직후 커밋)
    let (env, images) = world();
    env.uninstall_when_job_inserted(P, images[999]);

    // Act: 스레드가 스스로 끝나면 single-flight 자리가 비어 다시 시작된다 (그 두 번째 스레드도 곧바로 끝난다)
    assert!(started(&env));
    env.wait_until("first batch", |e| e.plugin_jobs() == 1_000);
    let (limit, mut freed) = (std::time::Instant::now() + Duration::from_secs(30), false);
    while !freed {
        assert!(std::time::Instant::now() < limit, "run-all thread did not exit after the uninstall");
        freed = started(&env);
        std::thread::yield_now();
    }
    env.jobs.stop();

    // Assert: 두 번째 배치 이후 행이 없다
    assert_eq!((env.plugin_jobs(), env.task_rows(P)), (1_000, 1_000));
    assert_eq!(env.cursor(P), None);
}

#[test]
fn t_pjob_19_export_provider_and_unusable_targets_are_rejected_without_starting_anything() {
    // Arrange
    let env = Env::new();
    env.install(exporter());
    env.install(scripted());
    env.bulk_assets(5, "image");

    // Act / Assert: 내보내기 provider 는 거부 (스레드·행 없음)
    assert!(matches!(env.jobs.run_all(EXPORTER, "exp.step"), Err(RunAllError::ExportProvider)));
    // 없는 provider / 없는 플러그인 / 꺼진 플러그인
    assert!(matches!(env.jobs.run_all(P, "nope"), Err(RunAllError::Run(RunError::UnknownTarget))));
    assert!(matches!(env.jobs.run_all("com.example.ghost", "x"), Err(RunAllError::Run(RunError::NotInstalled))));
    env.disable(P);
    assert!(matches!(env.jobs.run_all(P, "meta.run"), Err(RunAllError::Run(RunError::Disabled))));
    assert_eq!((env.task_rows(P), env.task_rows(EXPORTER), env.plugin_jobs()), (0, 0, 0));
    // 거부된 호출은 single-flight 자리를 차지하지 않는다
    env.enable(P);
    assert!(started(&env));
}
