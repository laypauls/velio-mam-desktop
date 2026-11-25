//! T-aij-12, 26~28 - 백그라운드 스윕(시간대·배치·배압), 시간대 경계, 동시성 설정, 쿼리 계획.
//! 분류: Time / Boundary / Concurrency(배압) / Business rule.

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use std::sync::Arc;

use ai_env::AiEnv;
use mam_core::SharedClock;
use mam_core::ai_jobs::*;
use mam_core::jobs::JobKind;

fn jobs(e: &AiEnv) -> i64 {
    e.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai'")
}

fn snap(e: &AiEnv) -> AiRuntimeSettings {
    e.settings.snapshot()
}

#[test]
fn t_aij_12_sweep_respects_schedule_window_and_only_takes_unanalyzed_up_to_the_batch() {
    // Arrange: T0 = 22:13 UTC. 예약 09~17 시
    let e = AiEnv::new();
    let ids: Vec<i64> = (0..5).map(|_| e.add_asset("image")).collect();
    e.set(|s| {
        s.background_analyze = true;
        s.schedule = Schedule {
            enabled: true,
            start_hour: 9,
            end_hour: 17,
        };
    });
    let clock: SharedClock = e.env.clock.clone();
    let sched = AiScheduler::new(Arc::clone(&e.env.pool), Arc::clone(&e.queue), clock, 3);

    // 시간대 밖(UTC 22시, KST 07시) -> 등록 안 함
    for off in [0, 540] {
        assert_eq!(
            sched.background_sweep(&snap(&e), off).unwrap(),
            QueueResult::default(),
            "offset {off}"
        );
    }
    assert_eq!(jobs(&e), 0);

    // 시간대 안(+660분 = 09시): 배치 상한 3
    let r = sched.background_sweep(&snap(&e), 660).unwrap();
    assert_eq!(r.queued, 3);
    assert_eq!(jobs(&e), 3);
    // 배압: 대기 작업이 이미 상한이면 더 넣지 않는다
    assert_eq!(sched.background_sweep(&snap(&e), 660).unwrap().queued, 0);
    // 하나 시작하면 한 건만 더 (이미 분석 대기 중인 자산은 미분석이 아니다)
    e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    assert_eq!(sched.background_sweep(&snap(&e), 660).unwrap().queued, 1);
    let queued_assets = e
        .env
        .count("SELECT COUNT(DISTINCT asset_id) FROM ai_task_state");
    assert_eq!(queued_assets, 4);
    assert_eq!(e.st(ids[4], "describe"), "none", "다섯 번째는 아직 미분석");

    // 꺼짐·동의 철회 -> 아무것도 하지 않는다
    e.set(|s| s.background_analyze = false);
    assert_eq!(
        sched.background_sweep(&snap(&e), 660).unwrap(),
        QueueResult::default()
    );
    e.set(|s| {
        s.background_analyze = true;
        s.consent_given = false;
    });
    assert_eq!(
        sched.background_sweep(&snap(&e), 660).unwrap(),
        QueueResult::default()
    );
}

#[test]
fn t_aij_12b_sweep_does_not_reselect_assets_with_nothing_to_do() {
    let e = AiEnv::new();
    let img = e.add_asset("image");
    e.set(|s| {
        s.background_analyze = true;
        s.external_kinds.clear();
        s.semantic_enabled = false;
    });

    let first = e.sched.background_sweep(&snap(&e), 0).unwrap();
    let second = e.sched.background_sweep(&snap(&e), 0).unwrap();

    assert_eq!((first.queued, first.skipped), (0, 1));
    assert_eq!(
        (second.queued, second.skipped),
        (0, 0),
        "표식이 있어 다시 고르지 않는다"
    );
    assert_eq!(
        e.step(img, "describe"),
        Some(("skipped".into(), Some("disabled".into())))
    );
}

#[test]
fn t_aij_26_schedule_window_and_local_hour_boundaries() {
    let off = Schedule::default();
    assert!((0..24).all(|h| off.allows(h)), "꺼져 있으면 항상");
    let all_day = Schedule {
        enabled: true,
        start_hour: 5,
        end_hour: 5,
    };
    assert!((0..24).all(|h| all_day.allows(h)), "시작 == 끝 = 하루 종일");
    let day = Schedule {
        enabled: true,
        start_hour: 9,
        end_hour: 17,
    };
    assert_eq!(
        [8, 9, 16, 17].map(|h| day.allows(h)),
        [false, true, true, false]
    );
    let night = Schedule {
        enabled: true,
        start_hour: 22,
        end_hour: 6,
    };
    assert_eq!(
        [21, 22, 23, 0, 5, 6].map(|h| night.allows(h)),
        [false, true, true, true, true, false],
        "자정 넘김"
    );
    assert_eq!(local_hour(common::env::T0, 0), 22);
    assert_eq!(local_hour(common::env::T0, 540), 7);
    assert_eq!(local_hour(0, -60), 23, "음수 오프셋은 전날로");
}

#[test]
fn t_aij_27_ai_concurrency_defaults_to_one_and_is_configurable() {
    let mut cfg = AiJobsConfig::new("cache".into());
    let default_caps = cfg.queue_caps();
    cfg.concurrency = 3;
    let caps = cfg.queue_caps();
    cfg.concurrency = 0;

    assert_eq!(default_caps[JobKind::Ai as usize], 1);
    assert_eq!(caps[JobKind::Ai as usize], 3);
    assert_eq!(cfg.queue_caps()[JobKind::Ai as usize], 1, "0 은 1 로");
    assert_eq!(
        caps[JobKind::Thumb as usize],
        JobKind::Thumb.default_concurrency()
    );
}

#[test]
fn t_aij_28_status_lists_all_kinds_and_query_plans_use_indexes() {
    let e = AiEnv::new();
    let id = e.add_asset("image");
    e.queue_all(id, Some(&[StepKind::Ocr]), false);
    let st = e.env.read(|c| list_status(c, id).unwrap());
    assert_eq!(st.len(), 6);
    assert_eq!(
        st.iter().find(|s| s.kind == StepKind::Ocr).unwrap().state,
        "queued"
    );
    assert_eq!(st.iter().filter(|s| s.state == "none").count(), 5);
    assert_eq!(
        serde_json::to_value(&st[0]).unwrap()["errorCode"],
        serde_json::Value::Null
    );

    let plan = |sql: &str, params: usize| -> String {
        e.env.read(|c| {
            let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let args = rusqlite::params_from_iter(std::iter::repeat_n(0i64, params));
            s.query_map(args, |r| r.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .join(" | ")
        })
    };
    let sweep = plan(UNANALYZED_SQL, 2);
    assert!(
        sweep.contains("SEARCH s USING") && !sweep.contains("SCAN s"),
        "{sweep}"
    );
    let failed = plan(FAILED_SQL, 0);
    assert!(failed.contains("idx_ai_task_state_kind_state"), "{failed}");
    let usage = plan(mam_core::ai_store::USAGE_SQL, 3);
    assert!(usage.contains("idx_ai_usage_at"), "{usage}");
    let runs = plan(
        "SELECT run_id FROM analysis_runs WHERE asset_id = 1 ORDER BY at DESC, run_id DESC LIMIT 10",
        0,
    );
    assert!(
        runs.contains("idx_analysis_runs_asset") && !runs.contains("TEMP B-TREE"),
        "{runs}"
    );
    let steps = plan(
        "SELECT kind, attempts FROM ai_task_state WHERE asset_id = 1 AND state IN ('queued','running')",
        0,
    );
    assert!(steps.contains("PRIMARY KEY"), "{steps}");
    let pending = plan(
        "SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'",
        0,
    );
    assert!(pending.contains("USING"), "{pending}");
}
