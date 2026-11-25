//! 장면 작업 종류·claim 게이트 — T-scene-30 (종류 정의·일시정지 영속), T-scene-31 (게이트).
//! 분류: Business rule / Boundary / Concurrency(실행 중 카운터). 실제 SQLite + 수동 시계, 실행기 없이 큐만 쓴다.

mod common;
mod scene_support;

use scene_support as scene_support_shim;

use std::sync::Arc;

use common::env::Env;
use mam_core::SharedClock;
use mam_core::jobs::{JobKind, JobOutcome, JobQueue, NewJob};
use mam_db::scene_cuts::{SCENE_JOB_KIND, SCENE_PRIORITY_BACKGROUND, SCENE_PRIORITY_USER};

const ALL: [JobKind; JobKind::COUNT] = JobKind::ALL;

fn queue(env: &Env) -> Arc<JobQueue> {
    let clock: SharedClock = env.clock.clone();
    JobQueue::new(Arc::clone(&env.pool), clock)
}

fn scene(asset: i64, priority: Option<i64>) -> NewJob {
    NewJob {
        priority,
        ..NewJob::for_asset(JobKind::Scene, asset)
    }
}

#[test]
fn t_scene_30_kind_definition_matches_mam_db_constants() {
    assert_eq!(JobKind::COUNT, 8);
    assert_eq!(JobKind::ALL.len(), JobKind::COUNT);
    assert_eq!(JobKind::Scene.as_str(), "scene");
    assert_eq!(JobKind::Scene.as_str(), SCENE_JOB_KIND);
    assert_eq!(JobKind::parse("scene"), Some(JobKind::Scene));
    assert_eq!(JobKind::parse("scenes"), None);
    assert_eq!(JobKind::Scene.default_priority(), SCENE_PRIORITY_BACKGROUND);
    assert_eq!(SCENE_PRIORITY_USER, 45);
    assert_eq!(JobKind::Scene.default_concurrency(), 1);
    // 종류마다 서로 다른 일시정지 비트 (AtomicU8 안에 모두 들어간다)
    let mut seen = 0u16;
    for k in ALL {
        let idx = ALL.iter().position(|x| *x == k).unwrap();
        assert!(idx < 8);
        seen |= 1 << idx;
    }
    assert_eq!(seen.count_ones() as usize, JobKind::COUNT);
    assert_eq!(serde_json::to_value(JobKind::Scene).unwrap(), "scene");
}

#[test]
fn t_scene_30_scene_is_the_lowest_priority_kind() {
    // Plugin(70)은 자동 실행 작업이라 장면 검출보다 뒤다 (P6 골격에서 종류 추가)
    for k in ALL
        .into_iter()
        .filter(|k| !matches!(k, JobKind::Scene | JobKind::Plugin))
    {
        assert!(
            k.default_priority() < JobKind::Scene.default_priority(),
            "{k:?}"
        );
    }
}

#[test]
fn t_scene_30_jobs_table_accepts_scene_and_counts_include_it() {
    // Arrange / Act - V16 이후 jobs.kind CHECK 가 'scene' 을 받는다
    let env = Env::new();
    let q = queue(&env);
    assert!(q.enqueue(scene(1, None)).unwrap());
    assert!(
        !q.enqueue(scene(1, None)).unwrap(),
        "duplicate pending is ignored"
    );
    // Assert
    let counts = q.counts().unwrap();
    assert_eq!(counts.len(), JobKind::COUNT);
    let (_, c) = counts.iter().find(|(k, _)| *k == JobKind::Scene).unwrap();
    assert_eq!(c.pending, 1);
    assert_eq!(
        env.count("SELECT priority FROM jobs WHERE kind = 'scene'"),
        SCENE_PRIORITY_BACKGROUND
    );
}

#[test]
fn t_scene_30_scene_pause_bit_survives_restart_and_only_blocks_scene() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(scene(1, None)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Hash, 2)).unwrap();
    q.pause(Some(JobKind::Scene)).unwrap();
    // Act - 재시작: 같은 DB 위의 새 큐
    let q2 = queue(&env);
    q2.recover().unwrap();
    q2.restore_pause().unwrap();
    // Assert
    assert_eq!(q2.paused_kinds(), vec![JobKind::Scene]);
    assert!(q2.is_paused(JobKind::Scene) && !q2.is_paused(JobKind::Hash));
    let j = q2.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(j.kind, JobKind::Hash);
    assert!(
        q2.claim_next(&ALL).unwrap().is_none(),
        "scene must not start"
    );
    q2.resume(Some(JobKind::Scene)).unwrap();
    assert_eq!(q2.claim_next(&ALL).unwrap().unwrap().kind, JobKind::Scene);
}

#[test]
fn t_scene_31_gate_blocks_background_scene_while_thumb_or_meta_runs_but_lets_user_requests_start() {
    for busy_kind in [JobKind::Thumb, JobKind::Meta] {
        // Arrange - 백그라운드(60) 와 사용자 요청(45) 이 대기, 썸네일/메타 1건 실행 중
        let env = Env::new();
        let q = queue(&env);
        q.enqueue(NewJob::for_asset(busy_kind, 100)).unwrap();
        let busy = q.claim_next(&ALL).unwrap().unwrap();
        assert_eq!(busy.kind, busy_kind);
        q.enqueue(scene(1, None)).unwrap();
        // Act / Assert - 백그라운드는 시작하지 않는다
        assert!(q.claim_next(&ALL).unwrap().is_none(), "{busy_kind:?}");
        // 사용자 요청(45) 은 시작한다
        q.enqueue(scene(2, Some(SCENE_PRIORITY_USER))).unwrap();
        let j = q.claim_next(&ALL).unwrap().unwrap();
        assert_eq!(
            (j.kind, j.asset_id, j.priority),
            (JobKind::Scene, Some(2), 45)
        );
        q.finish(&j, Ok(JobOutcome::Done)).unwrap();
        // 경계: 46 은 백그라운드로 취급한다
        q.enqueue(scene(3, Some(SCENE_PRIORITY_USER + 1))).unwrap();
        assert!(q.claim_next(&ALL).unwrap().is_none());
        // 바쁜 작업이 끝나면 대기 중이던 백그라운드 작업이 시작한다 (우선순위 순: 46 -> 60)
        q.finish(&busy, Ok(JobOutcome::Done)).unwrap();
        let next = q.claim_next(&ALL).unwrap().unwrap();
        assert_eq!((next.kind, next.asset_id), (JobKind::Scene, Some(3)));
    }
}

#[test]
fn t_scene_31_gate_is_open_when_idle_and_ignores_other_busy_kinds() {
    // Arrange - 해시·AI 가 실행 중이어도 게이트는 메타·썸네일만 본다
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Hash, 1)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Ai, 2)).unwrap();
    let a = q.claim_next(&ALL).unwrap().unwrap();
    let b = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!((a.kind, b.kind), (JobKind::Hash, JobKind::Ai));
    q.enqueue(scene(3, None)).unwrap();
    // Act / Assert
    let j = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(j.kind, JobKind::Scene);
    // 동시성 1 - 두 번째는 시작하지 않는다
    q.enqueue(scene(4, None)).unwrap();
    assert!(q.claim_next(&ALL).unwrap().is_none());
}

#[test]
fn t_scene_31_paused_thumb_kind_does_not_hold_the_gate_closed() {
    // Arrange - 썸네일이 대기(미실행)만 하고 있으면 게이트는 열려 있다 (실행 중 카운터 기준)
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 1)).unwrap();
    q.pause(Some(JobKind::Thumb)).unwrap();
    q.enqueue(scene(2, None)).unwrap();
    // Act / Assert
    let j = q.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(j.kind, JobKind::Scene);
}

// ── 재등록 청크·이벤트 페이로드 ─────────────────────────────

#[test]
fn t_scene_32_requeue_pending_runs_in_chunks_and_is_idempotent() {
    use scene_support_shim::{Fixture, Scene};
    let f = Fixture::new(Scene::Block, false);
    f.p.put("lib/v2.mp4", b"second video");
    f.p.put("lib/v3.mp4", b"third video");
    f.start();
    let ids: Vec<i64> = ["v.mp4", "v2.mp4", "v3.mp4"]
        .iter()
        .map(|n| f.p.asset_of(n))
        .collect();
    for id in &ids {
        let id = *id;
        f.p.env
            .pool
            .write(mam_db::Priority::High, move |c| {
                c.execute(
                    "INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES (?1, 'pending', 1)",
                    [id],
                )?;
                Ok(())
            })
            .unwrap();
    }
    // 청크 2 -> 트랜잭션 2번(2 + 1)으로 3건 모두 등록
    assert_eq!(
        mam_core::scene::requeue_pending_in_chunks(&f.p.env.pool, 5, 2).unwrap(),
        3
    );
    assert_eq!(f.scene_jobs(), 3);
    assert_eq!(
        f.p.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND priority = 60"),
        3
    );
    // 멱등 - 이미 작업이 있으면 더 등록하지 않는다
    assert_eq!(
        mam_core::scene::requeue_pending(&f.p.env.pool, 6).unwrap(),
        0
    );
    assert_eq!(f.scene_jobs(), 3);
    // 정확히 청크 크기만큼일 때도 끝난다 (다음 청크는 0건)
    f.p.shutdown();
}

#[test]
fn t_scene_updated_payload_serializes_as_camel_case_asset_id() {
    let v = serde_json::to_value(mam_core::scene::SceneUpdatedPayload { asset_id: 42 }).unwrap();
    assert_eq!(v, serde_json::json!({ "assetId": 42 }));
    assert_eq!(mam_core::scene::SCENE_UPDATED_EVENT, "scene://updated");
}
