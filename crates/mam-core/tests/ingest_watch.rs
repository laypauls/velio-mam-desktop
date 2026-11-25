//! 감시·볼륨 감지·주기 재스캔·파이프라인 통합 테스트 (T-watch-01 ~ 04, T-ingest-01).
//!
//! 분류: Happy (01, T-ingest-01) / External (02) / Business rule (03, 04) / Time (04).
//! 결정적 시험은 `handle_watch_now` 로 이벤트를 직접 주입하고(디바운스 단위 시험은 `watch::Debouncer`),
//! 실제 `notify` 경로는 폴링 대기(상한 시간)로 한 건만 확인한다.

mod common;

use std::fs;
use std::time::Duration;

use common::env::{Env, wait_until};
use mam_core::jobs::JobKind;
use mam_core::scan::{Availability, PathAction, ScanState, apply_path, scan_folder};
use mam_core::watch::WatchMsg;
use std::sync::atomic::AtomicBool;

fn scan(env: &Env, id: i64) {
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

fn event(env: &Env, folder_id: i64, rel: &str) {
    env.ingest
        .handle_watch_now(WatchMsg::Path {
            folder_id,
            path: env.path(rel),
        })
        .unwrap();
}

fn loc(env: &Env, name: &str) -> Option<(i64, String)> {
    env.read(|c| {
        c.query_row(
            "SELECT asset_id, presence FROM file_locations
             WHERE rel_path LIKE ?1 AND detached_at IS NULL",
            [format!("%{name}")],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .ok()
    })
}

fn registered(env: &Env) -> (i64, i64) {
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan(env, id);
    env.clock.advance(5_000);
    (id, id)
}

#[test]
fn t_watch_01_create_rename_and_delete_are_reflected_from_the_filesystem_state() {
    // Arrange
    let env = Env::new();
    let a = env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    let a_id = loc(&env, "a.jpg").unwrap().0;

    // Act / Assert 1: 새 파일
    env.write("lib/b.jpg", 2);
    event(&env, id, "lib/b.jpg");
    assert_eq!(loc(&env, "b.jpg").unwrap().1, "present");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 2);

    // 2: 이름 변경 = 옛 경로 사라짐 + 새 경로 생김 (어느 순서로 와도 같은 자산)
    env.clock.advance(1_000);
    fs::rename(&a, env.path("lib/renamed.jpg")).unwrap();
    event(&env, id, "lib/a.jpg");
    assert_eq!(loc(&env, "a.jpg").unwrap().1, "missing");
    event(&env, id, "lib/renamed.jpg");
    assert_eq!(loc(&env, "renamed.jpg"), Some((a_id, "present".into())));
    assert!(
        loc(&env, "a.jpg").is_none(),
        "old location detached by the move"
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets"),
        2,
        "no new asset for a rename"
    );

    // 3: 삭제
    env.clock.advance(1_000);
    fs::remove_file(env.path("lib/b.jpg")).unwrap();
    event(&env, id, "lib/b.jpg");
    assert_eq!(loc(&env, "b.jpg").unwrap().1, "missing");

    // 4: 같은 이벤트를 다시 받아도 멱등
    let revisions = env.count("SELECT SUM(revision) FROM media_assets");
    event(&env, id, "lib/b.jpg");
    assert_eq!(
        env.count("SELECT SUM(revision) FROM media_assets"),
        revisions
    );
}

#[test]
fn t_watch_01b_new_and_removed_directories_are_reflected_with_their_contents() {
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);

    // 새 폴더(안에 파일이 이미 있는 채 옮겨 넣어진 경우)
    env.write("lib/trip/day1/x.jpg", 2);
    env.write("lib/trip/day1/y.mp4", 3);
    event(&env, id, "lib/trip");
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3);
    let top = env.ingest.list_subfolders(id, None).unwrap();
    assert_eq!((top[0].name.as_str(), top[0].asset_count), ("trip", 2));
    assert_eq!(
        env.ingest.list_subfolders(id, Some("trip")).unwrap()[0].name,
        "day1"
    );

    // 폴더 삭제 -> 하위 모두 missing, 트리에서 제거
    env.clock.advance(1_000);
    fs::remove_dir_all(env.path("lib/trip")).unwrap();
    event(&env, id, "lib/trip");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'"),
        2
    );
    assert!(env.ingest.list_subfolders(id, None).unwrap().is_empty());
}

#[test]
fn t_watch_01c_events_outside_scope_are_ignored() {
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    let before = env.count("SELECT COUNT(*) FROM media_assets");
    env.write("lib/.hidden.jpg", 2);
    env.write("lib/notes.txt", 3);
    env.write("outside/o.jpg", 4);
    fs::create_dir_all(env.data.join("thumbnails")).unwrap();
    fs::write(env.data.join("thumbnails").join("t.jpg"), b"t").unwrap();
    let apply = |p: std::path::PathBuf| apply_path(env.ingest.env(), id, &p).unwrap();

    assert_eq!(apply(env.path("lib/.hidden.jpg")), PathAction::Ignored);
    assert_eq!(apply(env.path("lib/notes.txt")), PathAction::Ignored);
    assert_eq!(apply(env.path("outside/o.jpg")), PathAction::Ignored);
    assert_eq!(
        apply(env.path("lib")),
        PathAction::Ignored,
        "the root itself"
    );
    assert_eq!(
        apply(env.data.join("thumbnails").join("t.jpg")),
        PathAction::Ignored
    );
    assert_eq!(
        apply_path(env.ingest.env(), 9_999, &env.path("lib/a.jpg")).unwrap(),
        PathAction::Ignored,
        "unknown folder"
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), before);
}

#[test]
fn t_watch_01d_events_for_an_offline_volume_change_nothing() {
    let env = Env::new();
    let gone = env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    env.volumes.set_connected("vol-a", false);
    env.ingest.supervise_once().unwrap();
    fs::remove_file(&gone).unwrap();

    event(&env, id, "lib/a.jpg");

    assert_eq!(
        loc(&env, "a.jpg").unwrap().1,
        "present",
        "offline is not deletion"
    );
}

#[test]
fn t_watch_02_overflow_or_watch_error_schedules_a_deduplicated_rescan() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap();

    // Act: 넘침 메시지가 여러 번 와도 대기 작업은 하나
    for _ in 0..3 {
        env.ingest
            .handle_watch_now(WatchMsg::Rescan { folder_id: id })
            .unwrap();
    }

    // Assert
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"),
        1
    );
    assert_eq!(
        env.ingest.list_folders().unwrap()[0].scan_state,
        ScanState::Scanning
    );
}

#[test]
fn t_watch_03_volume_reconnect_goes_online_and_schedules_a_rescan() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap();

    // Act 1: 분리
    env.volumes.set_connected("vol-a", false);
    let down = env.ingest.supervise_once().unwrap();

    // Assert 1: offline 표시, 자산은 그대로, 수동 재스캔은 offline 오류
    assert_eq!(down.went_offline.len(), 1);
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.availability, Availability::Offline);
    assert_eq!(view.asset_count, 1);
    assert!(matches!(
        env.ingest.rescan_folder(id),
        Err(mam_core::scan::IngestError::Offline)
    ));
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs"),
        0,
        "no scan queued while offline"
    );
    assert!(env.ingest.supervise_once().unwrap().scheduled.is_empty());

    // Act 2: 재연결
    env.volumes.set_connected("vol-a", true);
    let up = env.ingest.supervise_once().unwrap();

    // Assert 2
    assert_eq!(up.went_online.len(), 1);
    assert_eq!(up.scheduled, vec![id]);
    assert_eq!(
        env.ingest.list_folders().unwrap()[0].availability,
        Availability::Online
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"),
        1
    );
}

#[test]
fn t_watch_04_periodic_rescan_is_scheduled_only_when_due_and_not_already_queued() {
    // Arrange: 기본 간격 6시간
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap();

    // Act / Assert: 방금 스캔했으면 예약하지 않는다
    assert!(env.ingest.supervise_once().unwrap().scheduled.is_empty());

    // 6시간이 지나면 예약, 두 번째 점검은 중복 예약하지 않는다
    env.clock.advance(6 * 3_600_000);
    assert_eq!(env.ingest.supervise_once().unwrap().scheduled, vec![id]);
    assert!(env.ingest.supervise_once().unwrap().scheduled.is_empty());

    // 실패한 작업이 있으면 자동으로 다시 만들지 않는다 (수동 재스캔만)
    env.pool
        .write(mam_db::Priority::Low, |c| {
            c.execute("UPDATE jobs SET status = 'failed'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(env.ingest.supervise_once().unwrap().scheduled.is_empty());
    env.ingest.rescan_folder(id).unwrap();
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"),
        0
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE status = 'pending'"),
        1
    );
}

#[test]
fn t_watch_05_first_scan_that_never_completed_is_rescheduled_at_startup() {
    // Arrange: 폴더 행만 있고 작업·스캔 기록이 없는 상태 (등록 직후 앱이 죽은 경우)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let (id, _) = registered(&env);
    env.pool
        .write(mam_db::Priority::Low, |c| {
            c.execute("DELETE FROM jobs", [])?;
            c.execute("UPDATE managed_folders SET last_full_scan_at = NULL", [])?;
            Ok(())
        })
        .unwrap();

    // Act
    let report = env.ingest.supervise_once().unwrap();

    // Assert
    assert_eq!(report.scheduled, vec![id]);
}

#[test]
fn t_ingest_01_started_service_scans_in_the_background_honors_pause_and_follows_the_watcher() {
    // Arrange: 전역 일시정지 상태에서 폴더를 등록한다
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.jpg", 2);
    env.ingest.start().unwrap();
    env.ingest.pause_jobs(Some(JobKind::Scan)).unwrap();
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act 1: 일시정지 중에는 시작하지 않는다
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 0);
    let summary = env.ingest.summary().unwrap();
    assert!(summary.paused_kinds.contains(&JobKind::Scan));
    assert_eq!(summary.counts[&JobKind::Scan].pending, 1);
    assert_eq!(
        env.ingest.list_folders().unwrap()[0].scan_state,
        ScanState::Paused
    );

    // Act 2: 재개 -> 백그라운드 스캔 완료
    let generation = env.ingest.generation();
    env.ingest.resume_jobs(None).unwrap();
    let done = wait_until(Duration::from_secs(30), || {
        env.count("SELECT COUNT(*) FROM media_assets") == 2
            && env.count("SELECT COUNT(*) FROM jobs") == 0
    });

    // Assert 2
    assert!(done, "scan should finish in the background");
    assert_ne!(
        env.ingest.generation(),
        generation,
        "progress listeners are notified"
    );
    let view = env.ingest.list_folders().unwrap().remove(0);
    assert_eq!((view.asset_count, view.scan_state), (2, ScanState::Idle));
    assert!(view.last_full_scan_at.is_some());
    // 자산 수 집계는 2초 캐시다 — 갱신될 때까지 기다린다
    assert!(wait_until(Duration::from_secs(10), || {
        env.ingest.summary().unwrap().total_assets == 2
    }));

    // Act 3: 실시간 감시 (디바운스 150ms) — 실제 notify 경로
    env.write("lib/sub/new.jpg", 9);
    // 디바운스는 주입 시계로 판단한다 — 폴링하는 동안 시계를 전진시킨다
    let seen = wait_until(Duration::from_secs(30), || {
        env.clock.advance(100);
        env.count("SELECT COUNT(*) FROM media_assets") == 3
    });
    assert!(seen, "watcher should pick up the new file");
    fs::remove_file(env.path("lib/sub/new.jpg")).unwrap();
    let missing = wait_until(Duration::from_secs(30), || {
        env.clock.advance(100);
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing'") == 1
    });
    assert!(missing, "watcher should mark the deleted file missing");

    // 종료 후에는 스레드가 남지 않고 풀을 닫을 수 있다
    env.ingest.shutdown();
    env.pool.shutdown();
    let _ = id;
}

#[test]
fn t_ingest_02_total_assets_cache_is_dropped_when_job_state_changes() {
    // 회귀: 첫 요약이 캐시한 0 이 스캔 완료 이벤트에 그대로 실려 UI 가 0 에 머무르던 결함
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.jpg", 2);
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    assert_eq!(env.ingest.summary().unwrap().total_assets, 0);

    scan(&env, id);
    env.ingest.jobs().notify(); // 작업 완료(finish)가 하는 일

    assert_eq!(env.ingest.summary().unwrap().total_assets, 2);
}
