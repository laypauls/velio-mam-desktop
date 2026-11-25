//! 폴더별 재스캔 정책이 점검(supervisor)에 연결된 동작 (T-rsched-01 ~ 09).
//!
//! 분류: Business rule (01 NULL = 전역 기본, 05 수동 정책에서도 재연결·넘침 재스캔, 06 중복 예약 없음, 08 손상 값) /
//! Time (02·03·07 시계·시간대 주입, 04 앱 시작 1회) / External (09 감지기 시작 실패 재시도).
//! 실제 SQLite + 수동 시계 + 가짜 볼륨. 시간대는 고정 +09:00.
#![cfg(windows)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::env::{Env, T0};
use mam_core::ingest::rescan_policy::FixedOffsetZone;
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::scan::scan_folder;
use mam_core::watch::{
    ChangeDetector, ChangeSink, DetectError, DetectorOptions, DetectorRegistry, OsEvents,
    ProbeResult, ScanOnly, WatchHandle, WatchMsg, WatchTarget,
};
use mam_core::{Blake3Hasher, WindowsFileIdentity};
use serde_json::{Value, json};

const HOUR: i64 = 3_600_000;
/// T0 = 2023-11-14 22:13:20 UTC = 2023-11-15 07:13:20 KST. 다음 03:00 KST 까지 19h46m40s
const TO_NEXT_0300: i64 = 71_200_000;

fn env_kst() -> Env {
    Env::with_config(|c, _| c.local_zone = Arc::new(FixedOffsetZone(9 * 3600)))
}

/// 폴더 등록 + 첫 스캔 완료(완료 시각 = T0) + 작업 비우기
fn registered(env: &Env) -> i64 {
    env.write("lib/a.jpg", 1);
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    clear_jobs(env);
    id
}

fn clear_jobs(env: &Env) {
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap();
}

fn policy(env: &Env, id: i64, v: Value) {
    env.ingest.set_folder_rescan_policy(id, Some(v)).unwrap();
}

fn tick(env: &Env) -> Vec<i64> {
    env.ingest.supervise_once().unwrap().scheduled
}

#[test]
fn t_rsched_01_folder_without_policy_follows_the_global_interval() {
    // Arrange
    let env = env_kst();
    let id = registered(&env);

    // Act / Assert: 정책 NULL -> 뷰에 정책 없음, 다음 예정 = 완료 + 6h (기본), 5h59m 에는 예약 없음
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.rescan_policy, None);
    assert_eq!(view.next_rescan_at, Some(T0 + 6 * HOUR));
    env.clock.advance(6 * HOUR - 1);
    assert!(tick(&env).is_empty());
    env.clock.advance(1);
    assert_eq!(tick(&env), vec![id]);
}

#[test]
fn t_rsched_02_one_hour_interval_fires_before_the_six_hour_default() {
    let env = env_kst();
    let id = registered(&env);
    policy(
        &env,
        id,
        json!({"trigger": {"kind": "interval", "hours": 1}}),
    );

    env.clock.advance(HOUR - 1);
    assert!(tick(&env).is_empty());
    env.clock.advance(1);

    assert_eq!(tick(&env), vec![id]);
}

#[test]
fn t_rsched_03_schedule_fires_once_per_slot() {
    // Arrange: 매일 03:00 KST
    let env = env_kst();
    let id = registered(&env);
    policy(
        &env,
        id,
        json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}}),
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.next_rescan_at, Some(T0 + TO_NEXT_0300));

    // Act / Assert: 슬롯 직전엔 없고, 슬롯이 지나면 1회
    env.clock.advance(TO_NEXT_0300 - 1);
    assert!(tick(&env).is_empty());
    env.clock.advance(1);
    assert_eq!(tick(&env), vec![id]);
    assert!(tick(&env).is_empty(), "already queued");

    // 스캔이 끝난 것으로 치면(완료 시각 갱신) 다음 날 슬롯까지 예약 없음
    clear_jobs(&env);
    let now = env.clock_now() + 60_000;
    env.clock.advance(60_000);
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            Ok(c.execute("UPDATE managed_folders SET last_full_scan_at = ?1", [now])?)
        })
        .unwrap();
    env.clock.advance(20 * HOUR);
    assert!(tick(&env).is_empty());
    env.clock.advance(5 * HOUR);
    assert_eq!(tick(&env), vec![id]);
}

#[test]
fn t_rsched_04_on_start_fires_on_the_first_tick_of_each_process_only() {
    // Arrange
    let env = env_kst();
    let id = registered(&env);
    policy(
        &env,
        id,
        json!({"trigger": {"kind": "manual"}, "onStart": true}),
    );

    // Act / Assert: 첫 점검에서만
    assert_eq!(tick(&env), vec![id]);
    clear_jobs(&env);
    assert!(tick(&env).is_empty());

    // 같은 DB 로 새로 시작한 Ingest(= 앱 재시작)는 다시 1회
    let restarted = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&env.pool),
            clock: env.clock.clone(),
            volumes: env.volumes.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig::default(),
    );
    assert_eq!(restarted.supervise_once().unwrap().scheduled, vec![id]);
    restarted.shutdown();
}

#[test]
fn t_rsched_05_manual_policy_schedules_nothing_but_reconnect_and_overflow_still_rescan() {
    // Arrange
    let env = env_kst();
    let id = registered(&env);
    policy(&env, id, json!({"trigger": {"kind": "manual"}}));
    env.clock.advance(1000 * HOUR);

    // Act / Assert: 정책에 의한 예약 없음, 다음 예정도 없음
    assert!(tick(&env).is_empty());
    assert_eq!(env.ingest.list_folders().unwrap()[0].next_rescan_at, None);

    // 감시 넘침 재스캔은 동작
    env.ingest
        .handle_watch_now(WatchMsg::Rescan { folder_id: id })
        .unwrap();
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"),
        1
    );
    clear_jobs(&env);

    // 볼륨 재연결 재스캔은 동작
    env.volumes.set_connected("vol-a", false);
    env.ingest.supervise_once().unwrap();
    env.volumes.set_connected("vol-a", true);
    assert_eq!(tick(&env), vec![id]);
}

#[test]
fn t_rsched_06_no_duplicate_when_pending_or_failed() {
    let env = env_kst();
    let id = registered(&env);
    policy(
        &env,
        id,
        json!({"trigger": {"kind": "interval", "hours": 1}}),
    );
    env.clock.advance(HOUR);

    assert_eq!(tick(&env), vec![id]);
    assert!(tick(&env).is_empty(), "pending");
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("UPDATE jobs SET status = 'failed'", [])?)
        })
        .unwrap();
    assert!(
        tick(&env).is_empty(),
        "failed jobs are retried manually only"
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 1);
}

#[test]
fn t_rsched_07_window_gates_new_starts() {
    // Arrange: 1시간 간격 + 22:00~06:00 KST (T0 = 07:13 KST — 시간대 밖)
    let env = env_kst();
    let id = registered(&env);
    policy(
        &env,
        id,
        json!({"trigger": {"kind": "interval", "hours": 1}, "window": {"start": "22:00", "end": "06:00"}}),
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.next_rescan_at, Some(T0 + 53_200_000));

    // Act / Assert: 시간대 밖(09:13)에서는 주기가 지나도 예약하지 않는다
    env.clock.advance(2 * HOUR);
    assert!(tick(&env).is_empty());

    // 22:30 KST (= 07:13:20 + 15h16m40s) 에는 예약
    env.clock.advance(55_000_000 - 2 * HOUR);
    assert_eq!(tick(&env), vec![id]);
}

#[test]
fn t_rsched_08_corrupt_stored_policy_behaves_like_the_global_default() {
    let env = env_kst();
    let id = registered(&env);
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            Ok(c.execute("UPDATE managed_folders SET rescan_policy = '{garbage'", [])?)
        })
        .unwrap();

    env.clock.advance(6 * HOUR - 1);
    assert!(tick(&env).is_empty());
    env.clock.advance(1);

    assert_eq!(tick(&env), vec![id]);
    let stored: Option<String> = env.read(|c| {
        c.query_row("SELECT rescan_policy FROM managed_folders", [], |r| {
            r.get(0)
        })
        .unwrap()
    });
    assert_eq!(
        stored.as_deref(),
        Some("{garbage"),
        "DB value is left untouched"
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.rescan_policy, None, "not JSON -> nothing to show");
}

/// 처음 몇 번은 시작에 실패하는 감지기
struct Flaky {
    fail: AtomicBool,
    starts: AtomicUsize,
}

struct Handle;
impl WatchHandle for Handle {}

struct FlakyDetector(Arc<Flaky>);

impl ChangeDetector for FlakyDetector {
    fn id(&self) -> &'static str {
        "flaky"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        _t: &WatchTarget,
        _o: &DetectorOptions,
        _s: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        self.0.starts.fetch_add(1, Ordering::SeqCst);
        if self.0.fail.load(Ordering::SeqCst) {
            return Err(DetectError::Start("share offline".into()));
        }
        Ok(Box::new(Handle))
    }
}

#[test]
fn t_rsched_09_detector_that_failed_to_start_is_retried_on_the_next_tick() {
    // Arrange
    let flaky = Arc::new(Flaky {
        fail: AtomicBool::new(true),
        starts: AtomicUsize::new(0),
    });
    let f2 = Arc::clone(&flaky);
    let env = Env::with_config(move |c, _| {
        c.detectors = Arc::new(DetectorRegistry::new(
            vec![
                Arc::new(FlakyDetector(Arc::clone(&f2))),
                Arc::new(OsEvents),
                Arc::new(ScanOnly),
            ],
            &["flaky", "os_events"],
        ));
    });
    env.write("lib/a.jpg", 1);
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    let view = |env: &Env| {
        env.ingest
            .list_folders()
            .unwrap()
            .into_iter()
            .find(|v| v.folder_id == id)
            .unwrap()
    };

    // Act / Assert: 실패 -> 다음 후보(os_events)로 폴백, 사유 노출
    let v = view(&env);
    assert_eq!(v.watch_active.as_deref(), Some("os_events"));
    assert_eq!(v.watch_fallback_key.as_deref(), Some("start_failed"));
    let after_add = flaky.starts.load(Ordering::SeqCst);
    assert!(after_add >= 1);

    // 점검마다 다시 시도한다
    env.ingest.supervise_once().unwrap();
    assert!(flaky.starts.load(Ordering::SeqCst) > after_add);

    // 복구되면 정착
    flaky.fail.store(false, Ordering::SeqCst);
    env.ingest.supervise_once().unwrap();
    let v = view(&env);
    assert_eq!(
        (v.watch_active.as_deref(), v.watch_fallback_key.clone()),
        (Some("flaky"), None)
    );
    let settled = flaky.starts.load(Ordering::SeqCst);
    env.ingest.supervise_once().unwrap();
    assert_eq!(
        flaky.starts.load(Ordering::SeqCst),
        settled,
        "settled watch is not restarted"
    );
}
