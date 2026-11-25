// 장면전환점 command 계약 시험 (T-ipc-scene-01 ~ 03)
// Categories: Happy (01 인자 이름·응답 camelCase, 03 요청·백필·개수 왕복) / Validation (01 잘못된 모양 거부, 03 assetIds 범위) /
// Business rule (02 오류 코드 매핑·내부 정보 비노출, 03 게이트: 비영상 skip·멱등)
use mam_db::{DbPool, SystemClock, migrate, open};
use serde_json::json;

use super::*;
use crate::error::ErrorCode;

const NOW: i64 = 1_700_000_000_000;

struct Fx {
    _dir: tempfile::TempDir,
    pool: Arc<DbPool>,
    ud: Arc<UserData>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    conn.execute_batch(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
           VALUES (1, 'video', 1, 0, 0, 0), (2, 'image', 1, 0, 0, 0), (3, 'video', 1, 0, 0, 0), (4, 'audio', 1, 0, 0, 0);
         INSERT INTO media_metadata (asset_id, duration_ms, fps) VALUES (1, 60000, 25.0);",
    )
    .unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let ud = Arc::new(UserData::new(Arc::clone(&pool), Arc::new(SystemClock)));
    Fx { _dir: dir, pool, ud }
}

/// command 본문과 같은 경로: `run_blocking` -> 서비스 호출 -> `CommandError` 변환
fn call<T: Send + 'static>(
    ud: &Arc<UserData>,
    f: impl FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::block_on(run_blocking(Arc::clone(ud), f))
}

fn scalar(pool: &DbPool, sql: &'static str) -> i64 {
    pool.read(|c| Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))?)).unwrap()
}

#[test]
fn t_ipc_scene_01_arguments_use_camel_case_and_reject_malformed_shapes() {
    // Arrange / Act: 옵션 — 전부 생략 가능
    let empty: SceneDetectOptions = serde_json::from_value(json!({})).unwrap();
    let full: SceneDetectOptions = serde_json::from_value(
        json!({"redetect": true, "restoreSuppressed": true, "sensitivity": "high"}),
    )
    .unwrap();

    // Assert
    assert!(!empty.redetect && !empty.restore_suppressed && empty.sensitivity.is_none());
    assert!(full.redetect && full.restore_suppressed);
    assert_eq!(full.sensitivity, Some(SceneSensitivity::High));
    for bad in [
        json!({"restore_suppressed": true}), // snake_case 는 조용히 무시되지 않고 거부
        json!({"sensitivity": "extreme"}),
        json!({"sensitivity": "High"}),
        json!({"sensitivity": 1}),
        json!({"redetect": "true"}),
        json!({"unknown": true}),
        json!(null),
    ] {
        assert!(serde_json::from_value::<SceneDetectOptions>(bad.clone()).is_err(), "{bad}");
    }
    // source: 소문자만
    for ok in ["auto", "user"] {
        assert!(serde_json::from_value::<CutSource>(json!(ok)).is_ok());
    }
    for bad in [json!("Auto"), json!("manual"), json!(1), json!(null)] {
        assert!(serde_json::from_value::<CutSource>(bad.clone()).is_err(), "{bad}");
    }
    // ms·assetId: 정수만 (소수·문자열은 command 진입 전에 거부)
    for bad in [json!(1.5), json!("1000"), json!(null), json!(true)] {
        assert!(serde_json::from_value::<i64>(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn t_ipc_scene_01_responses_serialize_to_the_contract_shapes() {
    // Arrange
    let fx = fixture();
    let none = call(&fx.ud, |u| u.scene_cuts_get(1)).unwrap();
    call(&fx.ud, |u| u.scene_cut_add(1, 5000)).unwrap();
    let with_cut = call(&fx.ud, |u| u.scene_cut_add(1, 20_000)).unwrap();

    // Act / Assert: 행 없음
    assert_eq!(
        serde_json::to_value(&none).unwrap(),
        json!({"assetId": 1, "state": "none", "sensitivity": null, "error": null, "cuts": [],
               "suppressedCount": 0, "updatedAt": null})
    );
    // 사용자 컷만 있는 행 (추출 전 표시 유지: state done + sensitivity null)
    let v = serde_json::to_value(&with_cut).unwrap();
    assert_eq!(v["state"], "done");
    assert_eq!(v["sensitivity"], json!(null));
    assert_eq!(v["cuts"], json!([{"ms": 5000, "source": "user"}, {"ms": 20000, "source": "user"}]));
    assert!(v["updatedAt"].is_i64());
    let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    for k in ["assetId", "state", "sensitivity", "error", "cuts", "suppressedCount", "updatedAt"] {
        assert!(keys.iter().any(|x| x == k), "missing {k}: {keys:?}");
    }
    assert_eq!(keys.len(), 7, "{keys:?}");
    assert_eq!(
        serde_json::to_value(SceneRequestResult { queued: 2, skipped: 1 }).unwrap(),
        json!({"queued": 2, "skipped": 1})
    );
    assert_eq!(
        serde_json::to_value(SceneBackfillResult { queued: 9 }).unwrap(),
        json!({"queued": 9})
    );
}

#[test]
fn t_ipc_scene_02_errors_map_to_contract_codes_without_internals() {
    // Arrange
    let fx = fixture();
    call(&fx.ud, |u| u.scene_cut_add(1, 1000)).unwrap();

    // Act
    let not_found = [
        call(&fx.ud, |u| u.scene_cuts_get(99)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cut_add(99, 1)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cut_remove(99, 1, CutSource::Auto)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cuts_clear_auto(99)).unwrap_err(),
    ];
    let invalid = [
        call(&fx.ud, |u| u.scene_cut_add(2, 1000)).unwrap_err(), // 이미지
        call(&fx.ud, |u| u.scene_cut_add(4, 1000)).unwrap_err(), // 음성
        call(&fx.ud, |u| u.scene_cut_add(1, -1)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cut_add(1, 60_001)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cut_remove(2, 1000, CutSource::User)).unwrap_err(),
        call(&fx.ud, |u| u.scene_cuts_clear_auto(2)).unwrap_err(),
    ];

    // Assert
    assert!(not_found.iter().all(|e| e.code() == ErrorCode::NotFound), "{not_found:?}");
    assert!(invalid.iter().all(|e| e.code() == ErrorCode::InvalidInput), "{invalid:?}");
    let msgs: Vec<String> = invalid.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    assert!(msgs[0].contains("mediaType") && msgs[2].contains("ms"), "{msgs:?}");
    for e in not_found.iter().chain(invalid.iter()) {
        let body = serde_json::to_string(e).unwrap();
        assert!(!body.contains("SELECT") && !body.contains("mam.sqlite") && !body.contains("asset_scene_cuts"), "{body}");
    }
    // 실패한 호출은 기존 컷을 바꾸지 않았다
    let kept = call(&fx.ud, |u| u.scene_cuts_get(1)).unwrap();
    assert_eq!(kept.cuts.len(), 1);
}

#[test]
fn t_ipc_scene_03_request_registers_user_priority_jobs_and_skips_non_videos() {
    // Arrange
    let fx = fixture();

    // Act
    let out = request_assets(&fx.pool, &[1, 2, 3, 4, 99, 1], &SceneDetectOptions::default(), NOW).unwrap();
    let again = request_assets(&fx.pool, &[1, 3], &SceneDetectOptions::default(), NOW + 1).unwrap();

    // Assert: 중복 id 는 한 번만, 비영상·없는 자산은 skipped, 재요청은 멱등
    assert_eq!(out, SceneRequestResult { queued: 2, skipped: 3 });
    assert_eq!(again, SceneRequestResult { queued: 2, skipped: 0 });
    assert_eq!(scalar(&fx.pool, "SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND priority = 45 AND status = 'pending'"), 2);
    let state = call(&fx.ud, |u| u.scene_cuts_get(1)).unwrap();
    assert_eq!(serde_json::to_value(&state).unwrap()["state"], "pending");
}

#[test]
fn t_ipc_scene_03_request_validates_the_asset_id_list_size() {
    // Arrange
    let fx = fixture();
    let too_many: Vec<i64> = (1..=10_001).collect();
    let max_ok: Vec<i64> = (1..=10_000).collect();

    // Act
    let empty = request_assets(&fx.pool, &[], &SceneDetectOptions::default(), NOW).unwrap_err();
    let over = request_assets(&fx.pool, &too_many, &SceneDetectOptions::default(), NOW).unwrap_err();
    let ok = request_assets(&fx.pool, &max_ok, &SceneDetectOptions::default(), NOW).unwrap();

    // Assert
    for e in [empty, over] {
        assert_eq!(CommandError::from(e).code(), ErrorCode::InvalidInput);
    }
    assert_eq!(ok.queued + ok.skipped, 10_000);
    assert_eq!(ok.queued, 2, "영상 1·3 만 등록");
}

#[test]
fn t_ipc_scene_03_redetect_options_flow_through_to_the_stored_request() {
    // Arrange: 1 번은 자동 추출 완료 + 사용자가 지운 컷이 있다
    let fx = fixture();
    fx.pool
        .write(Priority::Low, |c| {
            scene_cuts::replace_auto_tx(c, 1, &[10_000, 30_000], "medium", Some(25.0), 1).unwrap();
            Ok(())
        })
        .unwrap();
    call(&fx.ud, |u| u.scene_cut_remove(1, 10_000, CutSource::Auto)).unwrap();
    let plain = request_assets(&fx.pool, &[1], &SceneDetectOptions::default(), NOW).unwrap();

    // Act
    let opts = SceneDetectOptions { redetect: true, restore_suppressed: true, sensitivity: Some(SceneSensitivity::Low) };
    let redo = request_assets(&fx.pool, &[1], &opts, NOW).unwrap();

    // Assert: 옵션 없는 요청은 건너뛰고, 재검출은 등록 + 숨김 비움 + 요청 민감도 기록
    assert_eq!(plain, SceneRequestResult { queued: 0, skipped: 1 });
    assert_eq!(redo, SceneRequestResult { queued: 1, skipped: 0 });
    let got = call(&fx.ud, |u| u.scene_cuts_get(1)).unwrap();
    assert_eq!(got.suppressed_count, 0);
    assert_eq!(got.cuts.len(), 1, "새 결과가 오기 전까지 기존 자동 컷 유지");
    assert_eq!(
        scalar(&fx.pool, "SELECT COUNT(*) FROM asset_scene_cuts WHERE requested_sensitivity = 'low'"),
        1
    );
}

// --- 백필: 즉시 반환 + 백그라운드 등록 ---------------------------------------

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// 영상 2,100 개 (1 은 행 있음) 를 가진 픽스처 — 대상 2,101 건
fn backfill_fixture() -> Fx {
    let fx = fixture();
    fx.pool
        .write(Priority::Low, |c| {
            c.execute_batch(
                "WITH RECURSIVE n(i) AS (SELECT 1000 UNION ALL SELECT i + 1 FROM n WHERE i < 3099)
                 INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
                 SELECT i, 'video', 1, 0, 0, 0 FROM n;",
            )?;
            Ok(())
        })
        .unwrap();
    call(&fx.ud, |u| u.scene_cut_add(1, 1000)).unwrap();
    fx
}

fn run_with(
    fx: &Fx,
    running: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    before_batch: Arc<dyn Fn() + Send + Sync>,
) -> (BackfillRun, mpsc::Receiver<()>) {
    let (done_tx, done_rx) = mpsc::channel();
    let run = BackfillRun {
        pool: Arc::clone(&fx.pool),
        running: Arc::clone(running),
        stop: Arc::clone(stop),
        on_done: Arc::new(move || {
            let _ = done_tx.send(());
        }),
        before_batch,
        now: NOW,
    };
    (run, done_rx)
}

fn job_snapshot(pool: &DbPool) -> (i64, i64, i64) {
    (
        scalar(pool, "SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND priority = 60"),
        scalar(pool, "SELECT COUNT(*) FROM asset_scene_cuts WHERE state = 'pending'"),
        scalar(pool, "SELECT COUNT(*) FROM jobs WHERE kind = 'scene' AND asset_id = 1"),
    )
}

#[test]
fn t_ipc_scene_03_backfill_returns_before_registration_completes_and_finishes_in_the_background() {
    // Arrange: 첫 배치 직전에 막히는 등록 스레드
    let fx = backfill_fixture();
    let running = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let gate: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = release_rx.lock().unwrap().recv_timeout(Duration::from_secs(20));
    });
    let (run, done_rx) = run_with(&fx, &running, &stop, gate);

    // Act: 명령은 막힌 등록을 기다리지 않고 대상 수를 돌려준다
    let out = start_backfill(run).unwrap();

    // Assert: 반환 시점에는 아직 아무것도 등록되지 않았고, 등록 중 표시가 켜져 있다
    assert_eq!(out.queued, 2101);
    assert_eq!(scalar(&fx.pool, "SELECT COUNT(*) FROM jobs WHERE kind = 'scene'"), 0);
    assert!(running.load(Ordering::SeqCst));
    // 풀어 주면 끝까지 등록하고 표시를 내리며 대기열을 깨운다
    for _ in 0..4 {
        let _ = release_tx.send(());
    }
    done_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!running.load(Ordering::SeqCst));
    assert_eq!(job_snapshot(&fx.pool), (2101, 2101, 0));
    assert_eq!(backfill_target_count(&fx.pool).unwrap(), 0);
}

#[test]
fn t_ipc_scene_03_second_backfill_while_running_does_not_start_another_registration() {
    // Arrange
    let fx = backfill_fixture();
    let running = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let counter = Arc::clone(&started);
    let gate: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let _ = release_rx.lock().unwrap().recv_timeout(Duration::from_secs(20));
    });
    let (first, done_rx) = run_with(&fx, &running, &stop, Arc::clone(&gate));
    let (second, second_done) = run_with(&fx, &running, &stop, gate);

    // Act
    let a = start_backfill(first).unwrap();
    let b = start_backfill(second).unwrap();
    for _ in 0..4 {
        let _ = release_tx.send(());
    }
    done_rx.recv_timeout(Duration::from_secs(30)).unwrap();

    // Assert: 두 번째 호출은 현재 대상 수만 돌려주고 스레드·완료 알림을 만들지 않는다
    assert_eq!((a.queued, b.queued), (2101, 2101));
    assert!(second_done.try_recv().is_err());
    assert_eq!(started.load(Ordering::SeqCst), 4, "배치 3 + 마지막 빈 배치 1 — 한 스레드분");
    assert_eq!(job_snapshot(&fx.pool).0, 2101, "중복 없음");
    // 끝난 뒤의 호출은 대상이 없어 아무것도 시작하지 않는다
    let (third, third_done) = run_with(&fx, &running, &stop, Arc::new(|| {}));
    assert_eq!(start_backfill(third).unwrap().queued, 0);
    assert!(third_done.try_recv().is_err());
}

#[test]
fn t_ipc_scene_03_background_backfill_ends_in_the_same_state_as_the_synchronous_run() {
    // Arrange: 같은 데이터의 두 DB
    let sync_fx = backfill_fixture();
    let bg_fx = backfill_fixture();
    let running = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (sync_run, _rx) = run_with(&sync_fx, &running, &stop, Arc::new(|| {}));
    let (bg_run, done_rx) = run_with(&bg_fx, &Arc::new(AtomicBool::new(false)), &stop, Arc::new(|| {}));

    // Act
    let registered = backfill_all(&sync_run).unwrap();
    start_backfill(bg_run).unwrap();
    done_rx.recv_timeout(Duration::from_secs(30)).unwrap();

    // Assert
    assert_eq!(registered, 2101);
    assert_eq!(job_snapshot(&bg_fx.pool), job_snapshot(&sync_fx.pool));
    assert_eq!(job_snapshot(&bg_fx.pool), (2101, 2101, 0));
    // 멱등: 다시 돌려도 늘지 않는다
    assert_eq!(backfill_all(&sync_run).unwrap(), 0);
    assert_eq!(job_snapshot(&sync_fx.pool).0, 2101);
}

#[test]
fn t_ipc_scene_03_backfill_stops_between_batches_on_shutdown_and_a_rerun_completes() {
    // Arrange: 두 번째 배치 직전 훅에서 종료 신호 (세 번째 배치 전에 멈춘다)
    let fx = backfill_fixture();
    let running = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let (stop2, calls2) = (Arc::clone(&stop), Arc::clone(&calls));
    let gate: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if calls2.fetch_add(1, Ordering::SeqCst) == 1 {
            stop2.store(true, Ordering::SeqCst);
        }
    });
    let (run, done_rx) = run_with(&fx, &running, &stop, gate);

    // Act
    start_backfill(run).unwrap();
    done_rx.recv_timeout(Duration::from_secs(30)).unwrap();

    // Assert: 두 배치(2000건)까지만 등록되고 표시는 내려간다. 신호가 풀린 뒤 재실행하면 나머지를 채운다
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(!running.load(Ordering::SeqCst));
    assert_eq!(job_snapshot(&fx.pool).0, 2000);
    stop.store(false, Ordering::SeqCst);
    let (again, again_done) = run_with(&fx, &running, &stop, Arc::new(|| {}));
    assert_eq!(start_backfill(again).unwrap().queued, 101);
    again_done.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(job_snapshot(&fx.pool).0, 2101);
}
