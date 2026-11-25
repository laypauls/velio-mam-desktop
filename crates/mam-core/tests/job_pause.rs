//! 일시정지 상태 영속화 통합 테스트 (T-jobpause-01 ~ 06; 07 은 `mam-db/tests/job_pause.rs`).
//!
//! 분류: Happy (01, 02) / Business rule (03, 04) / Boundary (05) / External (06).
//! 실제 SQLite + 수동 시계. "재시작" = 같은 DB 위에 새 큐/Ingest 인스턴스(메모리 상태 없음)를 만든다.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::Env;
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::jobs::{
    Job, JobContext, JobError, JobExecutor, JobKind, JobOutcome, JobQueue, NewJob,
};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity};
use mam_db::Priority;

const ALL: [JobKind; JobKind::COUNT] = JobKind::ALL;

fn queue(env: &Env) -> Arc<JobQueue> {
    let clock: SharedClock = env.clock.clone();
    JobQueue::new(Arc::clone(&env.pool), clock)
}

/// 재시작한 큐: 같은 DB, 메모리 상태 없음 -> 시작 시 복원.
fn restarted(env: &Env) -> Arc<JobQueue> {
    let q = queue(env);
    q.recover().unwrap();
    q.restore_pause().unwrap();
    q
}

fn put_raw(env: &Env, raw: &'static str) {
    env.pool
        .write(Priority::High, move |c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES ('job_pause', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [raw],
            )?;
            Ok(())
        })
        .unwrap();
}

#[test]
fn t_jobpause_01_kind_pause_survives_restart_and_blocks_that_kind() {
    // Arrange
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Hash, 1)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 2)).unwrap();
    q.pause(Some(JobKind::Hash)).unwrap();

    // Act: 재시작
    let q2 = restarted(&env);

    // Assert
    assert_eq!(q2.paused_kinds(), vec![JobKind::Hash]);
    assert!(!q2.paused_all());
    let j = q2.claim_next(&ALL).unwrap().unwrap();
    assert_eq!(j.kind, JobKind::Thumb, "other kinds keep running");
    assert!(
        q2.claim_next(&ALL).unwrap().is_none(),
        "hash must not start"
    );
}

#[test]
fn t_jobpause_02_global_pause_survives_restart() {
    let env = Env::new();
    let q = queue(&env);
    q.enqueue(NewJob::for_asset(JobKind::Thumb, 1)).unwrap();
    q.enqueue(NewJob::for_asset(JobKind::Ai, 2)).unwrap();
    q.pause(None).unwrap();

    let q2 = restarted(&env);

    assert!(q2.paused_all());
    assert!(q2.claim_next(&ALL).unwrap().is_none(), "nothing may start");
}

#[test]
fn t_jobpause_03_resume_survives_restart() {
    // Arrange: 전역 + 종류별 일시정지 후 종류별 재개 -> 재시작 시 전역만 남음 -> 전체 재개 -> 해제
    let env = Env::new();
    let q = queue(&env);
    q.pause(None).unwrap();
    q.pause(Some(JobKind::Ai)).unwrap();
    q.pause(Some(JobKind::Hash)).unwrap();
    q.resume(Some(JobKind::Hash)).unwrap();
    let q2 = restarted(&env);
    assert!(q2.paused_all());
    assert_eq!(q2.paused_kinds(), vec![JobKind::Ai]);

    // Act
    q2.resume(None).unwrap();
    let q3 = restarted(&env);

    // Assert
    assert!(!q3.paused_all());
    assert!(q3.paused_kinds().is_empty());
}

struct Recording {
    runs: Arc<AtomicUsize>,
    tx: Mutex<Sender<()>>,
}

impl JobExecutor for Recording {
    fn execute(&self, _: &Job, _: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let _ = self.tx.lock().unwrap().send(());
        Ok(JobOutcome::Done)
    }
}

#[test]
fn t_jobpause_04_restore_happens_before_the_runner_starts_so_no_paused_job_runs() {
    // Arrange: 일시정지(hash) 저장 후 "재시작" - 새 Ingest 에 hash/thumb 실행기를 꽂는다
    let env = Env::new();
    env.ingest.jobs().pause(Some(JobKind::Hash)).unwrap();
    let clock: SharedClock = env.clock.clone();
    let ingest2 = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&env.pool),
            clock,
            volumes: env.volumes.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            supervise_every: Duration::from_secs(3600),
            ..IngestConfig::default()
        },
    );
    let (hash_runs, thumb_runs) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (tx, rx) = channel();
    for (kind, runs) in [(JobKind::Hash, &hash_runs), (JobKind::Thumb, &thumb_runs)] {
        ingest2.register_executor(
            kind,
            Arc::new(Recording {
                runs: Arc::clone(runs),
                tx: Mutex::new(tx.clone()),
            }),
        );
    }
    // hash 를 thumb 보다 먼저 고르도록 우선순위를 높인다: 일시정지가 적용되지 않았다면 첫 claim 이 hash 다
    ingest2
        .jobs()
        .enqueue(NewJob {
            priority: Some(1),
            ..NewJob::for_asset(JobKind::Hash, 1)
        })
        .unwrap();
    ingest2
        .jobs()
        .enqueue(NewJob::for_asset(JobKind::Thumb, 2))
        .unwrap();

    // Act: 시작 -> 일시정지되지 않은 thumb 이 실행될 때까지 채널로 기다린다(폴링 없음, 상한 있는 수신)
    ingest2.start().unwrap();
    rx.recv_timeout(Duration::from_secs(30))
        .expect("thumb job should run");
    ingest2.shutdown(); // 실행기 스레드를 모두 join - 이후 실행 횟수는 확정

    // Assert
    assert_eq!(thumb_runs.load(Ordering::SeqCst), 1);
    assert_eq!(hash_runs.load(Ordering::SeqCst), 0, "paused kind ran");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash' AND status = 'pending'"),
        1,
        "hash job was never claimed"
    );
}

#[test]
fn t_jobpause_05_missing_corrupt_and_unknown_values_restore_safely() {
    let env = Env::new();

    // 저장값 없음 -> 해제
    let q = restarted(&env);
    assert!(!q.paused_all() && q.paused_kinds().is_empty());

    // 깨진 JSON -> 해제 (앱 시작을 막지 않는다)
    put_raw(&env, "{broken");
    let q = restarted(&env);
    assert!(!q.paused_all() && q.paused_kinds().is_empty());

    // 모르는 종류 이름 -> 무시하고 나머지 적용
    put_raw(&env, r#"{"all":false,"kinds":["hash","future-kind","ai"]}"#);
    let q = restarted(&env);
    assert!(!q.paused_all());
    assert_eq!(q.paused_kinds(), vec![JobKind::Hash, JobKind::Ai]);
}

#[test]
fn t_jobpause_06_save_failure_returns_error_and_leaves_state_unchanged() {
    // Arrange: 일시정지(ai) 상태에서 settings 쓰기가 실패하도록 트리거를 건다
    let env = Env::new();
    let q = queue(&env);
    q.pause(Some(JobKind::Ai)).unwrap();
    env.pool
        .write(Priority::High, |c| {
            c.execute_batch(
                "CREATE TRIGGER fail_settings_write BEFORE INSERT ON settings
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;
                 CREATE TRIGGER fail_settings_update BEFORE UPDATE ON settings
                 BEGIN SELECT RAISE(ABORT, 'injected'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    // Act / Assert: 일시정지·재개 모두 오류, 메모리 상태 불변
    assert!(q.pause(None).is_err());
    assert!(!q.paused_all());
    assert!(q.resume(Some(JobKind::Ai)).is_err());
    assert!(q.resume(None).is_err());
    assert_eq!(q.paused_kinds(), vec![JobKind::Ai]);

    // 저장된 값도 그대로 - 재시작해도 같은 상태
    let q2 = restarted(&env);
    assert_eq!(q2.paused_kinds(), vec![JobKind::Ai]);
    assert!(!q2.paused_all());
}
