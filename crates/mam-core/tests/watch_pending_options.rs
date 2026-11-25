//! 안정화 게이트 옵션·시작 표식·스캔의 최근 수정 파일 처리 (T-pending-20 ~ 26).
//!
//! 분류: Business rule (20~22 배타적 열기 옵션 — 게이트 ON/OFF, 23 표식, 24 스캔 창) / Validation (21 옵션 검증) /
//! Boundary (24 미래 시각, 25 기존 자산) / 원본 보호 (읽기 전용 열기, 22 쓰기 핸들이 있는 파일).
//! 실제 SQLite + 임시 폴더 + 주입 시계. 파일 수정 시각은 주입 시계 기준으로 직접 정한다(픽스처만 만진다).
#![cfg(windows)]

mod common;

use std::fs::{self, OpenOptions};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use common::env::{Env, snapshot};
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps, PendingKind};
use mam_core::scan::{IngestError, scan_folder};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity};
use serde_json::json;

fn scan(env: &Env, id: i64) {
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

fn registered(env: &Env) -> i64 {
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    scan(env, id);
    env.clock.advance(5_000);
    id
}

fn assets(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM media_assets")
}

fn rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM watch_pending")
}

fn passes(env: &Env, n: usize) {
    for _ in 0..n {
        env.clock.advance(200);
        env.ingest.process_pending_once().unwrap();
    }
}

fn clear_jobs(env: &Env) {
    env.pool
        .write(mam_db::Priority::Low, |c| Ok(c.execute("DELETE FROM jobs", [])?))
        .unwrap();
}

fn marker_rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM settings WHERE key = 'ingest_run_marker'")
}

/// 주입 시계 기준 `delta_ms` 뒤(음수 = 전)로 수정 시각을 맞춘다 (픽스처만).
fn set_mtime(env: &Env, p: &std::path::Path, delta_ms: i64) {
    let t = SystemTime::UNIX_EPOCH + Duration::from_millis((env.clock_now() + delta_ms) as u64);
    OpenOptions::new().write(true).open(p).unwrap().set_modified(t).unwrap();
}

fn new_ingest(env: &Env) -> Arc<Ingest> {
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
            ..IngestConfig::default()
        },
    )
}

/// 쓰기 핸들이 열려 있는 파일을 만든다 (복사 도구가 쓰는 중인 상태).
fn writer_held(env: &Env, rel: &str) -> (std::path::PathBuf, fs::File) {
    let p = env.write(rel, 4);
    let h = OpenOptions::new().write(true).open(&p).unwrap();
    (p, h)
}

#[test]
fn t_pending_20_exclusive_open_check_on_keeps_a_file_pending_while_a_writer_holds_it() {
    // Arrange: 옵션 ON (게이트 ON 경로) — IPC 검증을 통과하고, 기본값(true)이라 저장되지 않는다
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let view = env
        .ingest
        .set_folder_watch_mode(id, Some("os_events".into()), Some(json!({"exclusiveOpenCheck": true})))
        .unwrap();
    assert_eq!(view.watch_options, None);
    let (p, handle) = writer_held(&env, "lib/w.jpg");
    let before = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();

    // Act: 표본은 몇 번이고 같지만 쓰기 핸들이 열려 있다
    passes(&env, 8);

    // Assert: 반영하지 않고 행을 유지
    assert_eq!(assets(&env), 1, "held-for-write file must not be applied");
    assert_eq!(rows(&env), 1);
    assert!(env.count("SELECT stable_count FROM watch_pending") >= 3);

    // Act 2: 쓰기 핸들을 놓는다
    drop(handle);
    passes(&env, 2);

    // Assert 2
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 0);
    let after = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    assert_eq!(before, after, "the probe opens read-only and never changes originals");
}

#[test]
fn t_pending_21_exclusive_open_check_on_by_default_and_explicit_off_applies_on_samples_alone() {
    // Arrange: 옵션 없음 (기본 켜짐) — 쓰기 핸들이 열린 파일
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    let (p, handle) = writer_held(&env, "lib/w.jpg");
    env.ingest.queue_watch_change(id, &p, PendingKind::Path).unwrap();

    // Act: 표본은 같지만 쓰기 핸들이 열려 있다
    passes(&env, 6);

    // Assert: 기본값만으로 반영을 미룬다
    assert_eq!(assets(&env), 1, "default-on check must hold a file open for write");
    assert_eq!(rows(&env), 1);

    // Act 2: 명시적으로 끄면 크기·수정 시각만으로 반영 (false 는 기본값이 아니므로 저장된다)
    let off = env
        .ingest
        .set_folder_watch_mode(id, Some("os_events".into()), Some(json!({"exclusiveOpenCheck": false})))
        .unwrap();
    assert_eq!(off.watch_options, Some(json!({"exclusiveOpenCheck": false})));
    passes(&env, 4);

    // Assert 2
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 0);
    drop(handle);

    // Validation: 불리언이 아닌 값·알 수 없는 키는 거부, 기본값(true)은 저장하지 않는다
    for bad in [json!({"exclusiveOpenCheck": "yes"}), json!({"exclusiveOpenCheck": 1}), json!({"nope": true})] {
        let r = env.ingest.set_folder_watch_mode(id, Some("os_events".into()), Some(bad.clone()));
        assert!(matches!(r, Err(IngestError::InvalidInput("watchOptions"))), "{bad}");
    }
    let on = env
        .ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), Some(json!({"exclusiveOpenCheck": true})))
        .unwrap();
    assert_eq!(on.watch_options, None, "default value is not stored");
    let off2 = env
        .ingest
        .set_folder_watch_mode(id, Some("scan_only".into()), Some(json!({"exclusiveOpenCheck": false})))
        .unwrap();
    assert_eq!(off2.watch_options, Some(json!({"exclusiveOpenCheck": false})), "accepted by any detector");
}

#[test]
fn t_pending_22_exclusive_open_check_does_not_block_a_directory_row() {
    // Arrange: 폴더 행은 배타적 열기 대상이 아니다 (파일만) — 옵션이 켜져 있어도 반영된다
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.ingest
        .set_folder_watch_mode(id, Some("os_events".into()), Some(json!({"exclusiveOpenCheck": true})))
        .unwrap();
    env.write("lib/d/x.jpg", 2);
    env.ingest.queue_watch_change(id, &env.path("lib/d"), PendingKind::Subtree).unwrap();

    // Act
    passes(&env, 6);

    // Assert
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 0);
}

#[test]
fn t_pending_23_unclean_shutdown_marker_schedules_one_rescan_for_realtime_folders_only() {
    // Arrange: 실시간(자동 -> os_events) 폴더 1개 + 재스캔만(scan_only) 폴더 1개
    let env = Env::new();
    env.write("lib1/a.jpg", 1);
    env.write("lib2/b.jpg", 2);
    let rt = env.ingest.add_folder(env.path("lib1").to_str().unwrap()).unwrap().folder_id;
    let so = env.ingest.add_folder(env.path("lib2").to_str().unwrap()).unwrap().folder_id;
    env.ingest.set_folder_watch_mode(so, Some("scan_only".into()), None).unwrap();
    clear_jobs(&env);

    // Act 1: 첫 실행 — 표식 없음 -> 예약 없음, 표식이 남는다
    let first = env.ingest.begin_run().unwrap();
    assert!(first.is_empty());
    assert_eq!(marker_rows(&env), 1);
    // 정상 종료하지 못한 채 다시 시작 (표식을 지우지 않는다)
    let restarted = new_ingest(&env);
    let scheduled = restarted.begin_run().unwrap();

    // Assert 1: 실시간 폴더만 재스캔 1회
    assert_eq!(scheduled, vec![rt]);
    assert_eq!(
        env.count(&format!("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending' AND folder_id = {rt}")),
        1
    );
    assert_eq!(
        env.count(&format!("SELECT COUNT(*) FROM jobs WHERE folder_id = {so}")),
        0
    );
    // 이미 대기 중이면 중복 예약하지 않는다
    assert!(new_ingest(&env).begin_run().unwrap().is_empty());

    // Act 2: 정상 종료 -> 표식이 지워지고 다음 시작은 예약하지 않는다
    clear_jobs(&env);
    restarted.end_run();
    assert_eq!(marker_rows(&env), 0);
    let clean = new_ingest(&env);
    assert!(clean.begin_run().unwrap().is_empty(), "clean shutdown leaves no marker");
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), 0);
    // begin_run 하지 않은 서비스의 종료는 남의 표식을 지우지 않는다
    new_ingest(&env).end_run();
    assert_eq!(marker_rows(&env), 1);
}

#[test]
fn t_pending_24_scan_defers_recently_modified_and_future_files_to_the_settle_gate() {
    // Arrange: 스캔의 최근 수정 창(60초) ON. 시계는 주입 시계 기준
    let env = Env::with_config(|c, _| c.pending.recent_guard_ms = 60_000);
    let old = env.write("lib/old.jpg", 1);
    let fresh = env.write("lib/fresh.jpg", 2);
    let future = env.write("lib/future.jpg", 3);
    set_mtime(&env, &old, -3_600_000);
    set_mtime(&env, &fresh, -5_000);
    set_mtime(&env, &future, 3_600_000);
    let before = snapshot(&env.vol);
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;

    // Act
    scan(&env, id);

    // Assert 1: 오래된 파일만 등록, 나머지는 source=scan 으로 대기열에
    assert_eq!(assets(&env), 1);
    assert_eq!(rows(&env), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE source = 'scan'"), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM scan_errors"), 0);

    // Act 2: 안정화 표본이 3회 같으면 반영된다 (시계 비교가 아니라 표본 비교 — 미래 시각 파일도 같다)
    passes(&env, 6);

    // Assert 2
    assert_eq!(assets(&env), 3);
    assert_eq!(rows(&env), 0);
    assert_eq!(snapshot(&env.vol), before, "originals unchanged (content + mtime)");
}

#[test]
fn t_pending_25_scan_does_not_mark_an_existing_asset_missing_when_its_file_was_just_modified() {
    // Arrange: 오래된 파일을 스캔으로 등록한 뒤, 내용을 바꾸고 수정 시각을 방금으로 둔다 (복사로 덮어쓰는 중)
    let env = Env::with_config(|c, _| c.pending.recent_guard_ms = 60_000);
    let f = env.write("lib/a.jpg", 1);
    set_mtime(&env, &f, -3_600_000);
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    scan(&env, id);
    assert_eq!(assets(&env), 1);
    env.clock.advance(1_000);
    fs::write(&f, vec![9u8; 500]).unwrap();
    set_mtime(&env, &f, -1_000);

    // Act: 재스캔 — 바뀐 파일은 반영하지 않고 대기열로
    scan(&env, id);

    // Assert 1: 기존 자산은 missing 도 superseded 도 아니다
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'present' AND detached_at IS NULL"),
        1
    );
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"), 1);
    assert_eq!(assets(&env), 1);
    assert_eq!(rows(&env), 1);

    // Act 2: 안정화 후 반영 -> 내용이 바뀐 것이므로 새 자산 (기존 규칙 D4)
    passes(&env, 6);

    // Assert 2
    assert_eq!(rows(&env), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"), 1);
}

#[test]
fn t_pending_26_guard_is_off_by_default_so_a_plain_scan_registers_fresh_files() {
    // 회귀: 기본 설정(0)에서는 방금 만든 파일도 스캔이 바로 등록한다 (기존 동작)
    let env = Env::new();
    env.write("lib/fresh.jpg", 1);
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;

    scan(&env, id);

    assert_eq!(assets(&env), 1);
    assert_eq!(rows(&env), 0);
}

#[test]
fn t_pending_27_exclusive_check_on_a_removable_volume_goes_through_the_guard_and_still_defers_a_held_file() {
    // Arrange: 이동식 볼륨 폴더 — 막힐 수 있는 볼륨이라 열기 검사가 제한 시간 호출을 거친다 (게이트 ON 경로)
    let env = Env::new();
    let dir = env.vol.parent().unwrap().join("usb");
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib").join("u.jpg"), [7u8; 90]).unwrap();
    env.volumes.add("usb", mam_core::VolumeKind::Removable, &dir);
    let id = env.ingest.add_folder(dir.join("lib").to_str().unwrap()).unwrap().folder_id;
    scan(&env, id);
    env.clock.advance(5_000);
    let held = dir.join("lib").join("w.jpg");
    fs::write(&held, [4u8; 80]).unwrap();
    let handle = OpenOptions::new().write(true).open(&held).unwrap();
    env.ingest.queue_watch_change(id, &held, PendingKind::Path).unwrap();
    let before = snapshot(&dir.join("lib")).into_iter().filter(|e| e.0 == "u.jpg").collect::<Vec<_>>();

    // Act: 쓰기 핸들이 열린 동안 점검을 여러 번
    passes(&env, 8);

    // Assert: 반영하지 않고 유지, 검사 스레드는 남지 않는다 (제한 시간 안에 끝났다)
    assert_eq!(assets(&env), 1, "held-for-write file must not be applied");
    assert_eq!(rows(&env), 1);
    let volume_id: i64 = env.read(|c| c.query_row("SELECT volume_id FROM managed_folders", [], |r| r.get(0)).unwrap());
    assert!(common::env::wait_until(Duration::from_secs(10), || env.ingest.env().io.live_checks(volume_id) == 0));

    // Act 2: 쓰기 핸들을 놓는다
    drop(handle);
    passes(&env, 2);

    // Assert 2: 반영되고 원본은 그대로
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 0);
    let after = snapshot(&dir.join("lib")).into_iter().filter(|e| e.0 == "u.jpg").collect::<Vec<_>>();
    assert_eq!(before, after);
}

#[test]
fn t_pending_28_due_order_and_per_folder_count_queries_use_the_pending_indexes() {
    // Arrange: 마이그레이션이 적용된 실제 DB
    let env = Env::new();
    let plan = |sql: &str, arg: i64| -> String {
        env.read(|c| {
            let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            s.query_map([arg], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect::<Vec<_>>().join(" | ")
        })
    };

    // Act: 처리기의 기한 순 조회(꺼내기 준비 폴더 제외 절 포함)와 폴더별 행 수 조회와 같은 모양
    let due = plan(
        "SELECT * FROM watch_pending WHERE due_at <= ?1 AND folder_id NOT IN (7,8) ORDER BY due_at, folder_id, path_key LIMIT 200",
        0,
    );
    let count = plan("SELECT COUNT(*) FROM watch_pending WHERE folder_id = ?1", 1);

    // Assert: 전체 스캔·임시 정렬 없이 인덱스/기본키로 푼다
    assert!(due.contains("idx_watch_pending_due"), "{due}");
    assert!(!due.contains("TEMP B-TREE"), "{due}");
    assert!(count.contains("SEARCH") && count.contains("folder_id=?"), "{count}");
}

#[test]
fn t_pending_34_shutdown_that_leaves_the_writer_undrained_keeps_the_marker_so_the_next_start_rescans() {
    // Arrange: 실제 스레드. DB 쓰기 스레드를 막아 기록기가 쓰기에서 멈추게 한다 (종료 기한을 넘기게)
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.ingest.start().unwrap();
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    assert!(common::env::wait_until(Duration::from_secs(30), || {
        assets(&env) == 1 && env.count("SELECT COUNT(*) FROM jobs") == 0
    }));
    assert_eq!(marker_rows(&env), 1, "a running service holds the marker");
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let pool = Arc::clone(&env.pool);
    let blocker = std::thread::spawn(move || {
        pool.write(mam_db::Priority::High, move |_| {
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(15));
            Ok(())
        })
        .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    env.write("lib/new.jpg", 2);
    // 이벤트가 기록기에 닿아 쓰기(flush)에서 막힐 시간 (기록 주기 250ms 의 몇 배) — 관찰할 수단이 없어 한 번만 기다린다
    std::thread::sleep(Duration::from_millis(1_500));

    // Act: 기한 300ms 로 종료 — 기록기가 못 끝낸다
    let abandoned = env.ingest.shutdown_within(Duration::from_millis(300));

    // Assert: 떼어 낸 스레드가 있고 표식은 남는다
    assert!(abandoned >= 1, "the blocked writer is abandoned");
    assert_eq!(marker_rows(&env), 1, "an undrained shutdown keeps the marker");

    // Act 2: 막힘을 풀고 다음 시작 — 실시간 폴더 재스캔 1회가 예약된다
    drop(release_tx);
    blocker.join().unwrap();
    clear_jobs(&env);
    let scheduled = new_ingest(&env).begin_run().unwrap();
    assert_eq!(scheduled, vec![id]);
    assert_eq!(
        env.count(&format!("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending' AND folder_id = {id}")),
        1
    );
}
