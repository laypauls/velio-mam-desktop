//! 변경 대기열(`watch_pending`) + 안정화 게이트 통합 테스트 (T-pending-01 ~ 14).
//!
//! 분류: Happy (02 복사 완료 후 1회 반영, 03 재시작 후 반영, 09 하위 폴더) / Boundary (01 같은 경로 1000회, 06 상한) /
//! Business rule (02 superseded 0건, 08 삭제는 missing, 10 꺼내기 중 행 유지, 11 폴더 제거 CASCADE) /
//! External (05 반영 실패 백오프·scan_errors) / Concurrency (07 처리 중 재보고) / 원본 보호 (02, 12).
//! 실제 SQLite + 임시 폴더 + 주입 시계. 파일 표본은 실제 파일시스템에서 읽고 시각만 시계로 정한다.
#![cfg(windows)]

mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::env::{Env, snapshot, wait_until};
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps, PendingConfig, PendingKind, SettleGate};
use mam_core::scan::{IngestError, PathAction, scan_folder};
use mam_core::{Blake3Hasher, SharedClock, VolumeKind, WindowsFileIdentity};
use std::sync::atomic::AtomicBool;

fn scan(env: &Env, id: i64) {
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

/// 폴더 `lib` 를 등록하고 한 번 스캔한다.
fn registered(env: &Env) -> i64 {
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    scan(env, id);
    env.clock.advance(5_000);
    id
}

fn pass(env: &Env) -> mam_core::ingest::PendingPassReport {
    env.ingest.process_pending_once().unwrap()
}

fn assets(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM media_assets")
}

fn rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM watch_pending")
}

fn clear_jobs(env: &Env) {
    env.pool
        .write(mam_db::Priority::Low, |c| Ok(c.execute("DELETE FROM jobs", [])?))
        .unwrap();
}

fn pending_scans(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'")
}

/// 제품 기본 게이트(표본 3회·간격 2초, 디바운스 2초)로 동작하는 환경
fn prod_gate(c: &mut IngestConfig, _: &std::path::Path) {
    c.debounce_ms = 2_000;
    c.pending.gate = SettleGate::default();
}

fn append(p: &std::path::Path, n: usize) {
    let mut f = OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(&vec![7u8; n]).unwrap();
}

#[test]
fn t_pending_01_same_path_reported_1000_times_is_one_row_and_a_rereport_restarts_the_settling() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let p = env.write("lib/b.jpg", 2);

    // Act: 1000 회 보고
    for _ in 0..1000 {
        env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();
    }

    // Assert: 1행
    assert_eq!(rows(&env), 1);
    assert_eq!(env.ingest.pending_count(id).unwrap(), 1);
    // 표본을 두 번 쌓은 뒤 다시 보고하면 연속 횟수가 0 으로 돌아가고 기한이 뒤로 밀린다
    env.clock.advance(200);
    pass(&env);
    env.clock.advance(100);
    pass(&env);
    assert_eq!(env.count("SELECT stable_count FROM watch_pending"), 2);
    let due_before = env.count("SELECT due_at FROM watch_pending");
    env.clock.advance(50);
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();
    assert_eq!(rows(&env), 1);
    assert_eq!(env.count("SELECT stable_count FROM watch_pending"), 0);
    assert!(env.count("SELECT due_at FROM watch_pending") > due_before);
    assert_eq!(assets(&env), 1, "nothing applied yet");
}

#[test]
fn t_pending_02_copy_that_stalls_over_two_seconds_yields_exactly_one_asset_and_no_superseded() {
    // Arrange: 제품 기본 게이트. 기존 파일 1개 + 복사 중인 파일
    let env = Env::with_config(prod_gate);
    let orig = env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let before = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    let copy = env.path("lib/big.jpg");
    fs::write(&copy, vec![9u8; 100]).unwrap();
    env.ingest.queue_watch_change(id, &copy, PendingKind::Path).unwrap();
    let step = |ms: i64| {
        env.clock.advance(ms);
        pass(&env)
    };

    // Act / Assert: 복사가 2 초 넘게 멈춘 동안에는 반영하지 않는다 (예전에는 디바운스 2 초 뒤 바로 등록됐다)
    assert_eq!(step(0).examined, 0, "not due yet");
    assert_eq!(step(2_000).waiting, 1);
    assert_eq!(step(2_000).waiting, 1);
    assert_eq!(assets(&env), 1, "stalled copy must not be registered");
    // 복사가 이어진다 -> 표본이 바뀌어 처음부터
    append(&copy, 50);
    assert_eq!(step(2_000).waiting, 1);
    assert_eq!(env.count("SELECT stable_count FROM watch_pending"), 1);
    append(&copy, 50);
    assert_eq!(step(2_000).waiting, 1);
    assert_eq!(assets(&env), 1);
    // 끝난 뒤 표본 3 회 연속 같으면 반영
    assert_eq!(step(2_000).waiting, 1);
    let done = step(2_000);

    // Assert: 자산 정확히 1개 추가, superseded 0, 행 삭제, 원본 불변
    assert_eq!(done.applied, 1);
    assert_eq!(assets(&env), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE file_size = 200"), 1);
    assert_eq!(rows(&env), 0);
    let after = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    assert_eq!(before, after, "original must be untouched");
    let _ = orig;
}

fn second_ingest(env: &Env) -> Arc<Ingest> {
    let clock: SharedClock = env.clock.clone();
    Ingest::new(
        IngestDeps {
            pool: Arc::clone(&env.pool),
            clock,
            volumes: env.volumes.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![env.data.clone()],
            debounce_ms: 150,
            pending: PendingConfig {
                gate: SettleGate { samples: 3, min_gap_ms: 100, give_up_ms: 600_000 },
                ..PendingConfig::default()
            },
            ..IngestConfig::default()
        },
    )
}

#[test]
fn t_pending_03_changes_recorded_before_a_clean_exit_are_applied_after_the_restart() {
    // Arrange: 실제 스레드(기록기·처리기)와 실제 notify 경로. 시계는 멈춰 둔다 -> 처리기는 행을 확인하지 않는다
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.ingest.start().unwrap();
    env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap();
    assert!(wait_until(Duration::from_secs(30), || {
        assets(&env) == 1 && env.count("SELECT COUNT(*) FROM jobs") == 0
    }));
    env.write("lib/new.jpg", 2);

    // Act 1: 기록기가 행을 쓴 뒤 종료 (종료 때 mpsc 의 남은 것도 마저 쓴다)
    assert!(
        wait_until(Duration::from_secs(30), || {
            env.count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%new.jpg'") == 1
        }),
        "the writer must record the event"
    );
    env.ingest.shutdown();

    // Assert 1: 자산은 아직 없고 행은 남았으며, 정상 종료라 표식은 지워졌다
    assert_eq!(assets(&env), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%new.jpg'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM settings WHERE key = 'ingest_run_marker'"), 0);

    // Act 2: 재시작 (같은 DB, 새 서비스) — 남은 행은 표본을 버리고 다시 확인한다
    env.pool
        .write(mam_db::Priority::Low, |c| {
            c.execute("UPDATE watch_pending SET stable_count = 2, sample_size = 1, sample_mtime = 1", [])?;
            Ok(())
        })
        .unwrap();
    let restarted = second_ingest(&env);
    let recovered = restarted.recover_pending().unwrap();
    assert!(recovered >= 1);
    assert_eq!(env.count("SELECT MAX(stable_count) FROM watch_pending"), 0, "recovery drops the old samples");
    for _ in 0..12 {
        env.clock.advance(200);
        restarted.process_pending_once().unwrap();
    }

    // Assert 2
    assert_eq!(assets(&env), 2, "the pending file is applied after the restart");
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%new.jpg'"), 0);
}

#[test]
fn t_pending_04_failing_apply_backs_off_and_is_recorded_in_scan_errors_after_five_attempts() {
    // Arrange: 항상 실패하는 반영 함수 (일시 오류가 계속되는 경우)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let p = env.write("lib/x.jpg", 2);
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();
    let fail = |_: &mam_core::scan::ScanEnv, _: i64, _: &std::path::Path| -> Result<PathAction, IngestError> {
        Err(IngestError::Io(std::io::Error::other("share is flaky")))
    };

    // Act: 표본 3 회 뒤 반영 시도 -> 실패마다 백오프 (표본 간격과 별개)
    let mut gave_up_at = None;
    let mut attempts_seen = Vec::new();
    for i in 0..14 {
        env.clock.advance(60_000);
        let r = env.ingest.process_pending_with(&fail).unwrap();
        if r.retried == 1 {
            attempts_seen.push(env.count("SELECT attempts FROM watch_pending"));
            assert_eq!(
                env.count("SELECT COUNT(*) FROM watch_pending WHERE last_error IS NOT NULL"),
                1,
                "the failure reason is kept on the row"
            );
        }
        if r.gave_up == 1 {
            gave_up_at = Some(i);
            break;
        }
    }

    // Assert: 1,2,3,4 번째 실패는 재시도, 5 번째에 scan_errors 로 남기고 행 삭제
    assert!(gave_up_at.is_some(), "must give up");
    assert_eq!(attempts_seen, vec![1, 2, 3, 4]);
    assert_eq!(rows(&env), 0);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM scan_errors WHERE rel_path = 'x.jpg' AND error_code = 'io' AND resolved_at IS NULL"),
        1
    );
    assert_eq!(assets(&env), 1, "nothing was applied");
}

#[test]
fn t_pending_05_unreadable_result_is_retried_but_not_recorded_twice() {
    // Arrange: apply_path 가 스스로 scan_errors 에 남기고 Failed 를 돌려주는 경우 — 처리기는 중복 기록하지 않는다
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let p = env.write("lib/x.jpg", 2);
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();
    let failed = |_: &mam_core::scan::ScanEnv, _: i64, _: &std::path::Path| -> Result<PathAction, IngestError> {
        Ok(PathAction::Failed)
    };

    // Act
    for _ in 0..14 {
        env.clock.advance(60_000);
        env.ingest.process_pending_with(&failed).unwrap();
    }

    // Assert
    assert_eq!(rows(&env), 0, "dropped after the attempt limit");
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_errors"), 0, "no duplicate record from the processor");
}

#[test]
fn t_pending_06_rows_over_the_folder_cap_turn_into_one_rescan() {
    // Arrange: 상한 3
    let env = Env::with_config(|c, _| c.pending.max_rows_per_folder = 3);
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    clear_jobs(&env);

    // Act: 서로 다른 경로 5개를 보고
    for n in 0..5 {
        env.ingest
            .queue_watch_change(id, &env.path(&format!("lib/n{n}.jpg")), PendingKind::Path)
            .unwrap();
    }

    // Assert: 3 행에서 멈추고 재스캔 1개 (반복 예약 없음). 이미 있는 경로의 재보고는 계속 받는다
    assert_eq!(rows(&env), 3);
    assert_eq!(pending_scans(&env), 1);
    clear_jobs(&env);
    env.ingest.queue_watch_change(id, &env.path("lib/n0.jpg"), PendingKind::Path).unwrap();
    env.ingest.queue_watch_change(id, &env.path("lib/n9.jpg"), PendingKind::Path).unwrap();
    assert_eq!(rows(&env), 3);
    assert_eq!(pending_scans(&env), 0, "the fallback rescan is requested once per overflow episode");

    // 행이 비어 상한 아래로 내려오면 다음 넘침은 다시 재스캔으로 바뀐다
    for _ in 0..4 {
        env.clock.advance(200);
        pass(&env);
    }
    assert_eq!(rows(&env), 0);
    for n in 0..5 {
        env.ingest
            .queue_watch_change(id, &env.path(&format!("lib/m{n}.jpg")), PendingKind::Path)
            .unwrap();
    }
    assert_eq!(pending_scans(&env), 1);
}

#[test]
fn t_pending_07_a_report_that_arrives_while_applying_is_not_lost() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let p = env.write("lib/b.jpg", 2);
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();
    for _ in 0..2 {
        env.clock.advance(200);
        pass(&env);
    }
    // 반영(3 번째 표본) 도중에 같은 경로가 다시 보고된다
    let ingest = Arc::clone(&env.ingest);
    let clock = Arc::clone(&env.clock);
    let path = p.clone();
    let rereport = move |e: &mam_core::scan::ScanEnv, f: i64, abs: &std::path::Path| -> Result<PathAction, IngestError> {
        let action = mam_core::scan::apply_path(e, f, abs)?;
        clock.advance(1);
        ingest.queue_watch_change(f, &path, PendingKind::Path)?;
        Ok(action)
    };

    // Act
    env.clock.advance(200);
    let r = env.ingest.process_pending_with(&rereport).unwrap();

    // Assert: 반영은 됐지만 새 보고는 행으로 남아 다시 확인된다 (조용히 지워지지 않는다)
    assert_eq!(r.applied, 1);
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 1);
    assert_eq!(env.count("SELECT stable_count FROM watch_pending"), 0);
}

#[test]
fn t_pending_08_a_delete_event_still_reaches_apply_path_and_marks_the_asset_missing() {
    // Arrange
    let env = Env::new();
    let a = env.write("lib/a.jpg", 1);
    let id = registered(&env);
    fs::remove_file(&a).unwrap();
    env.ingest.queue_watch_change(id, &a, PendingKind::Path).unwrap();

    // Act: 삭제는 표본을 기다리지 않고 첫 확인에서 바로 반영한다
    env.clock.advance(200);
    let r = pass(&env);

    // Assert
    assert_eq!(r.applied, 1);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing' AND detached_at IS NULL"),
        1
    );
    assert_eq!(assets(&env), 1, "the asset is kept (missing, not deleted)");
    assert_eq!(rows(&env), 0);
    // 같은 삭제를 다시 받아도 멱등
    let revisions = env.count("SELECT SUM(revision) FROM media_assets");
    env.ingest.queue_watch_change(id, &a, PendingKind::Path).unwrap();
    env.clock.advance(200);
    pass(&env);
    assert_eq!(env.count("SELECT SUM(revision) FROM media_assets"), revisions);
}

#[test]
fn t_pending_09_subtree_rows_walk_the_directory_after_it_settles() {
    // Arrange: 새 폴더(안에 파일이 이미 있는 채 옮겨 넣어진 경우)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.write("lib/trip/day1/x.jpg", 2);
    env.write("lib/trip/day1/y.mp4", 3);
    env.ingest
        .queue_watch_change(id, &env.path("lib/trip"), PendingKind::Subtree)
        .unwrap();

    // Act: 폴더가 안정된 뒤 순회가 찾은 파일도 같은 게이트를 한 번 더 거친다 (그래서 점검이 더 필요하다)
    for _ in 0..12 {
        env.clock.advance(200);
        pass(&env);
    }

    // Assert
    assert_eq!(assets(&env), 3);
    assert_eq!(rows(&env), 0);
    assert_eq!(env.ingest.list_subfolders(id, None).unwrap()[0].name, "trip");
}

fn plug_usb(env: &Env) -> (PathBuf, i64) {
    let dir = env.vol.parent().unwrap().join("usb");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib").join("u.jpg"), [7u8; 90]).unwrap();
    env.volumes.add("usb", VolumeKind::Removable, &dir);
    let id = env.ingest.add_folder(dir.join("lib").to_str().unwrap()).unwrap().folder_id;
    (dir, id)
}

#[test]
fn t_pending_10_rows_of_a_folder_held_for_eject_are_skipped_but_kept() {
    // Arrange: usb 폴더 + vol-a 폴더에 각각 대기 행
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let a = registered(&env);
    let (dir, usb) = plug_usb(&env);
    scan(&env, usb);
    let u_new = dir.join("lib").join("u2.jpg");
    fs::write(&u_new, [5u8; 80]).unwrap();
    let a_new = env.write("lib/b.jpg", 2);
    env.ingest.queue_watch_change(usb, &u_new, PendingKind::Path).unwrap();
    env.ingest.queue_watch_change(a, &a_new, PendingKind::Path).unwrap();

    // Act: usb 꺼내기 준비 중에 점검을 여러 번
    env.ingest.prepare_volume_eject(usb).unwrap();
    let mut examined = 0;
    for _ in 0..6 {
        env.clock.advance(200);
        examined += pass(&env).examined;
    }

    // Assert: vol-a 의 행은 반영되고, 붙들린 usb 의 행은 건드리지 않고 남는다
    assert_eq!(env.ingest.pending_count(usb).unwrap(), 1);
    assert_eq!(env.ingest.pending_count(a).unwrap(), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets"), 3, "a.jpg, u.jpg, b.jpg — not u2.jpg");
    assert!(examined >= 1);
    assert_eq!(env.count("SELECT stable_count FROM watch_pending"), 0, "held rows are not even sampled");
}

#[test]
fn t_pending_11_removing_a_folder_deletes_its_pending_rows_by_cascade() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.ingest.queue_watch_change(id, &env.path("lib/n.jpg"), PendingKind::Path).unwrap();
    assert_eq!(rows(&env), 1);

    // Act
    env.ingest.remove_folder(id).unwrap();

    // Assert: 제거가 끝나면 행도 사라진다. 제거 중(`enabled = 0`)에는 새 보고를 받지 않는다
    assert!(wait_until(Duration::from_secs(30), || {
        env.count("SELECT COUNT(*) FROM managed_folders") == 0
    }));
    assert_eq!(rows(&env), 0);
    assert!(env.ingest.queue_watch_change(id, &env.path("lib/n2.jpg"), PendingKind::Path).is_ok());
    assert_eq!(rows(&env), 0, "reports for a removed folder are dropped");
}

#[test]
fn t_pending_12_settling_never_changes_the_original_files() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/c.jpg", 3);
    let id = registered(&env);
    let b = env.write("lib/b.jpg", 2);
    let before = snapshot(&env.vol);
    env.ingest.queue_watch_change(id, &b, PendingKind::Path).unwrap();
    env.ingest.queue_watch_change(id, &env.path("lib/sub"), PendingKind::Subtree).unwrap();

    // Act
    for _ in 0..6 {
        env.clock.advance(200);
        pass(&env);
    }

    // Assert
    assert_eq!(assets(&env), 3);
    assert_eq!(snapshot(&env.vol), before, "content and mtime of every fixture are unchanged");
}
