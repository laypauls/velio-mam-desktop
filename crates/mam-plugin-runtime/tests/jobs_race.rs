//! P5 경합 (T-pjob-08 / 10) — 피드 중 동시 outbox 쓰기, 제거 경합.
//!
//! 분류: Concurrency(08 "작은 event_id 가 나중에 커밋되지 않는다", 10 제거 경합) / Business rule(10 꺼진 플러그인은 공급 안 함).
//! 실제 SQLite 의 쓰기 스레드 하나 위에서 사용자 편집(High) 스레드들과 피드(Low)가 섞인다.

mod common_jobs;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common_jobs::{Env, Plugin};
use mam_db::DbPool;
use mam_plugin_runtime::jobs::JobsTuning;
use mam_plugin_runtime::{AssetKind, ProviderKind};

const P: &str = "com.example.race";

fn scripted() -> Plugin {
    Plugin::new(P, "scripted").provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
}

/// (커서, 커서 이하 이벤트 중 작업에 반영되지 않은 수) — 한 읽기 연결에서 커서를 먼저 읽는다
fn unreflected(pool: &DbPool) -> (i64, i64) {
    pool.read(|c| {
        let cursor = mam_db::plugin_tasks::feed_cursor(c, P)?.unwrap_or(0);
        let missing = c.query_row(
            "SELECT COUNT(*) FROM search_outbox o JOIN media_assets a ON a.asset_id = o.asset_id AND a.status = 'active'
             WHERE o.event_id <= ?1 AND o.event_type = 'upsert'
               AND NOT EXISTS (SELECT 1 FROM plugin_tasks t WHERE t.plugin_ref = (SELECT plugin_ref FROM plugins WHERE plugin_id = ?2) AND t.provider_ref = (SELECT v.provider_ref FROM plugin_providers v WHERE v.plugin_ref = t.plugin_ref AND v.name = 'meta.run')
                               AND t.asset_id = o.asset_id AND t.target_revision >= o.revision)",
            (cursor, P),
            |r| r.get(0),
        )?;
        Ok((cursor, missing))
    })
    .unwrap()
}

#[test]
fn t_pjob_08_concurrent_outbox_writes_during_feed_ticks_never_leave_an_event_behind_the_cursor() {
    // Arrange: 자산 10개, 편집 스레드 4개(스레드당 5바퀴 x 10자산 = 200 이벤트), 작은 배치로 피드가 자주 끼어든다
    let env = Env::new();
    env.install(scripted());
    env.jobs.set_tuning(JobsTuning {
        feed_batch: 7,
        ..JobsTuning::default()
    });
    let assets: Vec<i64> = (0..10)
        .map(|i| env.add_asset("image", &format!("{i}.jpg")))
        .collect();
    let stop = Arc::new(AtomicBool::new(false));
    let feeder = {
        let (jobs, pool, stop) = (
            Arc::clone(&env.jobs),
            Arc::clone(&env.pool),
            Arc::clone(&stop),
        );
        std::thread::spawn(move || {
            let mut ticks = 0u32;
            loop {
                jobs.feed_tick();
                // 불변식: 어느 시점에 봐도 커서 이하의 모든 이벤트는 작업에 반영돼 있다
                let (cursor, missing) = unreflected(&pool);
                assert_eq!(
                    missing, 0,
                    "events at or below cursor {cursor} must be reflected"
                );
                ticks += 1;
                if stop.load(Ordering::SeqCst) {
                    return ticks;
                }
                std::thread::yield_now();
            }
        })
    };

    // Act
    let writers: Vec<_> = (0..4)
        .map(|_| {
            let (editor, assets) = (env.editor(), assets.clone());
            std::thread::spawn(move || {
                for _ in 0..5 {
                    for a in &assets {
                        editor.edit(*a);
                    }
                }
            })
        })
        .collect();
    writers.into_iter().for_each(|w| w.join().unwrap());
    stop.store(true, Ordering::SeqCst);
    let ticks = feeder.join().expect("the invariant held on every tick");

    // 정지 상태에서 남은 구간을 마저 공급한다 (틱은 읽은 시점의 끝까지만 가므로 쓰기가 끝난 뒤 한 번 더 필요하다)
    let mut guard = 0;
    while env.cursor(P) != Some(env.max_event()) {
        env.jobs.feed_tick();
        guard += 1;
        assert!(guard < 100, "feed must catch up once writers stop");
    }

    // Assert: 정지 상태에서 모든 이벤트가 커서 이하이고 반영됨 — 자산마다 최종 revision 이 target
    assert!(ticks >= 2);
    assert_eq!(env.max_event(), 200);
    assert_eq!(unreflected(&env.pool), (200, 0));
    assert!(assets.iter().all(|a| {
        env.task(P, "meta.run", *a)
            .is_some_and(|t| t.target == env.revision(*a) && t.target == 21)
    }));
}

#[test]
fn t_pjob_10_plugin_removed_mid_flight_leaves_no_kv_task_or_cursor_writes() {
    // Arrange: provider 가 돈 직후, 결과 기록 전에 제거(설정 행 삭제)가 끼어든다 (런타임 항목은 아직 남아 있다)
    let env = Env::new();
    env.install(Plugin::new(P, "ok_writer").provider(
        "meta.extract",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.jobs.feed_tick();
    let remove = env.settings_deleter(P);
    env.invoker.set_after(move |_| remove());

    // Act
    let done = env.run(1);

    // Assert: 기록은 `NotInstalled` 로 막혔다 — KV·태그·완료 모두 없고 작업은 그대로
    assert!(matches!(
        done[0].outcome,
        Ok(mam_core::jobs::JobOutcome::Done)
    ));
    assert_eq!(
        (env.kv_rows(), env.plugin_tags(asset), env.revision(asset)),
        (0, vec![], 2)
    );
    let task = env.task(P, "meta.extract", asset).unwrap();
    assert_eq!(
        (
            task.status.as_str(),
            task.done,
            task.attempts,
            task.last_error
        ),
        ("pending", 0, 0, None)
    );
    // 피드도 제거된 플러그인에는 아무것도 쓰지 않는다 (커서 재생성·작업 갱신 없음)
    env.edit(asset);
    env.jobs.feed_tick();
    assert_eq!(env.cursor(P), None);
    assert_eq!(env.task(P, "meta.extract", asset).unwrap().target, 2);
}

#[test]
fn t_pjob_10_feed_writes_nothing_when_only_the_installed_row_is_gone_and_a_disabled_plugin_is_not_fed()
 {
    // Arrange: 커서는 있지만 `plugin_installed` 만 사라진 경우
    let env = Env::new();
    env.install(scripted());
    let asset = env.add_asset("image", "a.jpg");
    env.edit(asset);
    env.exec(&format!(
        "DELETE FROM settings WHERE key = 'plugin_installed:{P}'"
    ));

    // Act
    env.jobs.feed_tick();

    // Assert: 작업·커서 이동 없음
    assert_eq!((env.task_rows(P), env.cursor(P)), (0, Some(0)));

    // Arrange 2: 정상 설치된 다른 플러그인을 끈다 -> 공급되지 않고 커서도 멈춘다. 켜면 밀린 이벤트를 따라잡는다
    let other = "com.example.disabled";
    env.install(Plugin::new(other, "scripted").provider(
        "meta.run",
        ProviderKind::Metadata,
        &[AssetKind::Image],
    ));
    env.disable(other);
    env.edit(asset);
    env.jobs.feed_tick();
    let stalled = (env.task_rows(other), env.cursor(other));
    env.enable(other);
    env.jobs.feed_tick();

    // Assert 2
    assert_eq!(stalled, (0, Some(1)));
    assert_eq!(
        (env.task_rows(other), env.cursor(other)),
        (1, Some(env.max_event()))
    );
}
