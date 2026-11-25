//! 하위 폴더 보고(`WatchMsg::Subtree`)와 폴더 순회의 안정화 게이트 연결 (T-subtree-02 ~ 08).
//!
//! 분류: Happy (02 기록기 경유 행, 04 하위 폴더 발견·출처 보존) / Business rule (03 자라는 파일은 안정될 때까지 등록 안 함·superseded 0,
//! 06 폴더 삭제는 missing, 07 즉시 처리 경로는 그대로) / Boundary (05 폴더 행 상한 -> 재스캔 1회) / Concurrency (08 Path·Subtree 합치기).
//! 실제 SQLite + 임시 폴더 + 주입 시계. 수정 시각을 보존하는 복사(탐색기·robocopy)는 파일 시각을 시계 기준 1시간 전으로 고정해 흉내 낸다.
#![cfg(windows)]

mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use common::env::{Env, T0, snapshot, wait_until};
use mam_core::ingest::{PendingKind, SCAN_RECENT_GUARD_MS, SettleGate};
use mam_core::scan::scan_folder;
use mam_core::watch::{
    ChangeDetector, ChangeSink, DetectError, DetectorOptions, DetectorRegistry, ProbeResult, WatchHandle, WatchMsg,
    WatchTarget,
};

fn scan(env: &Env, id: i64) {
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
}

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

/// 제품 기본 게이트(표본 3회·간격 2초, 디바운스 2초) + 스캔 최근 수정 창(60초)
fn prod(c: &mut mam_core::ingest::IngestConfig, _: &Path) {
    c.debounce_ms = 2_000;
    c.pending.gate = SettleGate::default();
    c.pending.recent_guard_ms = SCAN_RECENT_GUARD_MS;
}

/// 수정 시각을 시계 기준 1시간 전으로 고정한다 — 60초 최근 창을 피하는 "시각을 보존하는 복사"
fn keep_old_mtime(p: &Path) {
    let f = OpenOptions::new().write(true).open(p).unwrap();
    let old = u64::try_from(T0 - 3_600_000).unwrap();
    f.set_modified(UNIX_EPOCH + Duration::from_millis(old)).unwrap();
}

fn grow(p: &Path, n: usize) {
    let mut f = OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(&vec![7u8; n]).unwrap();
    drop(f);
    keep_old_mtime(p);
}

/// 시작할 때 받은 sink 를 보관하는 감지기 (시험이 감지기 대신 보고한다)
struct Capture(Arc<Mutex<Option<ChangeSink>>>);
struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Capture {
    fn id(&self) -> &'static str {
        "capture"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        _t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        *self.0.lock().unwrap() = Some(sink);
        Ok(Box::new(Handle))
    }
}

#[test]
fn t_subtree_02_a_subtree_report_is_recorded_by_the_writer_as_a_subtree_row_and_wins_over_a_path_report() {
    // Arrange: 실제 기록기 스레드 + 가짜 감지기
    let slot = Arc::new(Mutex::new(None));
    let detectors = Arc::new(DetectorRegistry::new(vec![Arc::new(Capture(Arc::clone(&slot)))], &["capture"]));
    let env = Env::with_config(move |c, _| c.detectors = detectors);
    env.write("lib/a.jpg", 1);
    env.ingest.start().unwrap();
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    assert!(wait_until(Duration::from_secs(30), || slot.lock().unwrap().is_some()));
    let sink = slot.lock().unwrap().clone().unwrap();
    let dir = env.path("lib/trip");
    fs::create_dir_all(&dir).unwrap();

    // Act: 같은 경로를 Path 로 먼저, Subtree 로 나중에 보고 (같은 묶음 안)
    sink.path(dir.clone());
    sink.subtree(dir.clone());
    sink.emit(WatchMsg::Path { folder_id: id, path: dir.clone() });

    // Assert: 1행, kind = subtree, 출처 = 감지기 ID
    assert!(wait_until(Duration::from_secs(30), || {
        env.count("SELECT COUNT(*) FROM watch_pending WHERE kind = 'subtree' AND abs_path LIKE '%trip'") == 1
    }));
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%trip'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE source = 'capture' AND abs_path LIKE '%trip'"), 1);
    env.ingest.shutdown();
}

#[test]
fn t_subtree_03_a_dropped_folder_with_a_growing_inner_file_registers_once_when_stable_and_never_supersedes() {
    // Arrange: 기존 자산 1개 + 폴더째 옮겨 넣는 중 (안쪽 파일은 시각을 보존하며 자란다 — 60초 창을 피한다)
    let env = Env::with_config(prod);
    keep_old_mtime(&env.write("lib/a.jpg", 1)); // 스캔 최근 수정 창 밖이어야 첫 스캔에서 바로 등록된다
    let id = registered(&env);
    let original = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    let big = env.path("lib/drop/sub/big.jpg");
    fs::create_dir_all(big.parent().unwrap()).unwrap();
    fs::write(&big, vec![9u8; 100]).unwrap();
    keep_old_mtime(&big);
    env.ingest.queue_watch_change(id, &env.path("lib/drop"), PendingKind::Subtree).unwrap();

    // Act 1: 파일이 계속 자라는 동안 점검을 반복한다
    for i in 0..12 {
        env.clock.advance(2_000);
        pass(&env);
        if i < 10 {
            grow(&big, 50);
        }
        assert_eq!(assets(&env), 1, "step {i}: a growing copy must not be registered");
    }
    assert!(rows(&env) >= 1, "the inner file is waiting in the gate");
    assert_eq!(
        env.count("SELECT COUNT(*) FROM watch_pending WHERE kind = 'path' AND abs_path LIKE '%big.jpg' AND source <> 'scan'"),
        1,
        "the walk queued the file as a path row and kept the reporter as its source"
    );

    // Act 2: 복사가 끝났다 — 안정되면 반영
    for _ in 0..20 {
        env.clock.advance(2_000);
        pass(&env);
        if assets(&env) == 2 {
            break;
        }
    }

    // Assert: 자산 정확히 1개 추가(최종 크기), superseded 0, 대기열 비움, 원본 불변
    assert_eq!(assets(&env), 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE file_size = 600"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"), 0);
    for _ in 0..4 {
        env.clock.advance(2_000);
        pass(&env);
    }
    assert_eq!(rows(&env), 0);
    assert_eq!(assets(&env), 2, "no extra asset after the queue drains");
    let after = snapshot(&env.vol.join("lib")).into_iter().filter(|e| e.0 == "a.jpg").collect::<Vec<_>>();
    assert_eq!(original, after, "original must be untouched");
}

#[test]
fn t_subtree_04_new_nested_directories_are_still_discovered_and_their_files_wait_in_the_gate() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.write("lib/trip/day1/deep/x.jpg", 2);
    env.write("lib/trip/day2/y.jpg", 3);
    env.ingest.queue_watch_change(id, &env.path("lib/trip"), PendingKind::Subtree).unwrap();

    // Act: 폴더 행이 안정될 때까지 (파일 행은 아직 반영 전)
    let mut queued_files = 0;
    for _ in 0..6 {
        env.clock.advance(200);
        pass(&env);
        queued_files = env.count("SELECT COUNT(*) FROM watch_pending WHERE kind = 'path'");
        if queued_files > 0 {
            break;
        }
    }

    // Assert: 하위 폴더는 발견돼 트리에 기록되고, 파일 2개는 자산이 되기 전에 행으로 대기한다
    assert_eq!(queued_files, 2);
    assert_eq!(assets(&env), 1, "files are not applied directly by the directory walk");
    assert_eq!(env.count("SELECT COUNT(*) FROM folder_dirs WHERE rel_path IN ('trip/day1/deep', 'trip/day2')"), 2);
    // 이어서 게이트를 통과하면 둘 다 반영
    for _ in 0..10 {
        env.clock.advance(200);
        pass(&env);
    }
    assert_eq!(assets(&env), 3);
    assert_eq!(rows(&env), 0);
}

#[test]
fn t_subtree_05_a_folder_over_the_row_cap_falls_back_to_exactly_one_rescan_and_loses_no_file() {
    // Arrange: 폴더당 행 상한 3 — 폴더 행 1 + 파일 10 개
    let env = Env::with_config(|c, _| c.pending.max_rows_per_folder = 3);
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    for i in 0..10u8 {
        env.write(&format!("lib/dump/f{i}.jpg"), 10 + i);
    }
    clear_jobs(&env);
    env.ingest.queue_watch_change(id, &env.path("lib/dump"), PendingKind::Subtree).unwrap();

    // Act: 폴더 행이 안정돼 순회가 일어날 때까지
    for _ in 0..4 {
        env.clock.advance(200);
        pass(&env);
        if pending_scans(&env) > 0 {
            break;
        }
    }

    // Assert: 재스캔 1회로 바뀌고(상한 초과분은 행으로 받지 않는다), 직접 반영하지 않았다
    assert_eq!(pending_scans(&env), 1);
    assert!(rows(&env) <= 3, "the cap holds");
    assert_eq!(assets(&env), 1);
    // 그 재스캔이 모두 맞춘다 — 대기열에 남은 행이 뒤늦게 처리돼도 중복 자산은 없다
    scan(&env, id);
    assert_eq!(assets(&env), 11);
    for _ in 0..10 {
        env.clock.advance(200);
        pass(&env);
    }
    assert_eq!(assets(&env), 11);
}

#[test]
fn t_subtree_06_a_subtree_whose_directory_vanished_marks_everything_below_it_missing() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/gone/x.jpg", 2);
    env.write("lib/gone/in/y.jpg", 3);
    let id = registered(&env);
    fs::remove_dir_all(env.path("lib/gone")).unwrap();
    env.ingest.queue_watch_change(id, &env.path("lib/gone"), PendingKind::Subtree).unwrap();

    // Act: 삭제는 표본을 기다리지 않는다
    env.clock.advance(200);
    let r = pass(&env);

    // Assert
    assert_eq!(r.applied, 1);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM file_locations WHERE presence = 'missing' AND detached_at IS NULL"),
        2
    );
    assert_eq!(assets(&env), 3, "assets are kept (missing, not deleted)");
    assert_eq!(rows(&env), 0);
}

#[test]
fn t_subtree_07_handle_watch_now_applies_a_subtree_report_directly_like_a_path_report() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.write("lib/new/x.jpg", 2);

    // Act: 디바운스·게이트 없이 처리하는 시험·진단 경로
    env.ingest.handle_watch_now(WatchMsg::Subtree { folder_id: id, dir: env.path("lib/new") }).unwrap();

    // Assert
    assert_eq!(assets(&env), 2);
    assert_eq!(rows(&env), 0);
}

#[test]
fn t_subtree_08_the_same_directory_reported_again_restarts_its_settling_and_stays_one_row() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let id = registered(&env);
    env.write("lib/trip/x.jpg", 2);
    let dir = env.path("lib/trip");

    // Act: Subtree 다음 Path — 종류는 subtree 로 유지된다 (UPSERT 합치기)
    env.ingest.queue_watch_change(id, &dir, PendingKind::Subtree).unwrap();
    env.ingest.queue_watch_change(id, &dir, PendingKind::Path).unwrap();

    // Assert
    assert_eq!(rows(&env), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM watch_pending WHERE kind = 'subtree'"), 1);
}
