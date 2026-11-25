//! 큰 파일 하나를 해시하는 동안의 멈춤 판정 (T-stall-09 ~ 11) — 경로는 그대로여도 바이트가 진행하면 멈춘 것이 아니다.
//!
//! 분류: Boundary (09 3분 경계) / Business rule (09 진행 중이면 4분이 지나도 정상, 10 바이트가 멈추면 3분에 멈춤) /
//! Time (시계 주입) / 통합 (11 실제 스캔의 해시 읽기 경로가 진행을 알린다).
//! 실제 SQLite + 임시 폴더. 09·10 은 진행 상태를 직접 조작하고, 11 은 스캔이 부르는 해시 계산기를 대역으로 갈아 끼운다.
#![cfg(windows)]

mod common;

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::env::{AtomicClock, Env};
use mam_core::scan::{ScanEnv, scan_folder};
use mam_core::{Blake3Hasher, ContentHasher, CoreError, HashedFile};

const THREE_MIN: i64 = 180_000;
const STEP: i64 = 30_000;

fn stalled(env: &Env, id: i64) -> bool {
    env.ingest.scan_stalls().contains_key(&id)
}

fn scanning_one_file(env: &Env) -> i64 {
    env.write("lib/big.mov", 1);
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    let p = env.ingest.env();
    p.progress.start(id, r"X:\lib".into(), 0, env.clock_now());
    p.progress.hold(id, r"X:\lib\big.mov", env.clock_now());
    id
}

#[test]
fn t_stall_09_steady_byte_progress_on_one_file_is_never_a_stall_even_after_four_minutes() {
    // Arrange: 같은 경로를 계속 붙들고 있다
    let env = Env::new();
    let id = scanning_one_file(&env);

    // Act / Assert: 4분 동안 30초마다 바이트가 읽힌다 -> 계속 정상
    for _ in 0..8 {
        env.clock.advance(STEP);
        env.ingest.env().progress.bytes_read(id, env.clock_now());
        assert!(!stalled(&env, id));
    }
    // 마지막 진행 뒤 2:59.999 까지 정상, 정확히 3분에 멈춤 (경계 포함)
    env.clock.advance(THREE_MIN - 1);
    assert!(!stalled(&env, id));
    env.clock.advance(1);
    assert!(stalled(&env, id));
}

#[test]
fn t_stall_10_no_bytes_for_three_minutes_is_a_stall_and_a_new_byte_recovers_it() {
    // Arrange
    let env = Env::new();
    let id = scanning_one_file(&env);
    let held_since = env.clock_now();

    // Act: 바이트가 3분 동안 없다
    env.clock.advance(THREE_MIN);

    // Assert: 붙든 경로 그대로 멈춤 표시, 시작 시각은 마지막 진행 시각
    let s = env.ingest.scan_stalls().remove(&id).expect("stalled");
    assert_eq!(
        (s.at_path.as_str(), s.since),
        (r"X:\lib\big.mov", held_since)
    );
    // 바이트가 다시 읽히면 풀린다 (처리 건수는 늘지 않았다)
    env.ingest.env().progress.bytes_read(id, env.clock_now());
    assert!(!stalled(&env, id));
    assert_eq!(env.ingest.env().progress.all()[0].processed, 0);
}

/// 해시 도중 시계를 30초씩 4분 전진시키며(report 면 그때마다 바이트 진행을 알리며) 멈춤 판정을 기록하는 해시 계산기 (대역)
struct SlowHasher {
    /// 환경이 만들어진 뒤 연결한다 (환경 시계·진행 상태)
    attached: Mutex<Option<(Arc<AtomicClock>, Arc<ScanEnv>)>>,
    report: bool,
    reports: AtomicUsize,
    stalled_during: AtomicBool,
    stalled_at_end: AtomicBool,
}

impl ContentHasher for SlowHasher {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError> {
        Blake3Hasher.hash_file(path, cancel)
    }

    fn hash_file_progress(
        &self,
        path: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<HashedFile, CoreError> {
        let (clock, env) = self.attached.lock().unwrap().clone().expect("attached");
        for i in 1..=8u64 {
            clock.advance(STEP);
            if self.report {
                on_bytes(i * 1_000_000);
                self.reports.fetch_add(1, Ordering::SeqCst);
            }
            let now = mam_db::Clock::now_ms(&*clock);
            let stall = env.progress.all().iter().any(|p| p.stall_at(now).is_some());
            if stall && i < 8 {
                self.stalled_during.store(true, Ordering::SeqCst);
            }
            if stall && i == 8 {
                self.stalled_at_end.store(true, Ordering::SeqCst);
            }
        }
        Blake3Hasher.hash_file(path, cancel)
    }
}

/// 해시를 아는 자산의 파일 수정 시각이 바뀌면(터치) 스캔이 전체 해시로 "내용이 같은가" 를 확인한다 — 그 해시를 대역이 맡는다.
fn scan_a_touched_file(report: bool) -> Arc<SlowHasher> {
    let hasher = Arc::new(SlowHasher {
        attached: Mutex::new(None),
        report,
        reports: AtomicUsize::new(0),
        stalled_during: AtomicBool::new(false),
        stalled_at_end: AtomicBool::new(false),
    });
    let env = Env::with_deps(
        |_, _| {},
        Arc::new(mam_core::WindowsFileIdentity),
        hasher.clone(),
    );
    *hasher.attached.lock().unwrap() = Some((Arc::clone(&env.clock), Arc::clone(env.ingest.env())));
    let p = env.path("lib/a.mov");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let content = vec![5u8; 20_000];
    std::fs::write(&p, &content).unwrap();
    let id = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    let hash = blake3::hash(&content).as_bytes().to_vec();
    env.pool
        .write(mam_db::Priority::Low, move |c| {
            Ok(c.execute("UPDATE media_assets SET content_hash = ?1, hash_algo = 'blake3', hash_state = 'done'", [hash])?)
        })
        .unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
        .unwrap();
    drop(f);
    env.clock.advance(1_000);
    scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    hasher
}

#[test]
fn t_stall_11_the_real_hash_path_reports_bytes_so_a_four_minute_hash_is_not_a_stall() {
    // Arrange / Act: 스캔이 부르는 해시가 4분 걸리지만 바이트는 계속 진행한다
    let hasher = scan_a_touched_file(true);

    // Assert: 진행을 알렸고, 4분이 지나도 멈춤으로 판정되지 않았다
    assert!(
        hasher.reports.load(Ordering::SeqCst) >= 8,
        "the hash path must report progress"
    );
    assert!(!hasher.stalled_during.load(Ordering::SeqCst));
    assert!(
        !hasher.stalled_at_end.load(Ordering::SeqCst),
        "4 min with steady bytes is not a stall"
    );
}

#[test]
fn t_stall_12_a_hash_that_reports_no_bytes_is_a_stall_at_three_minutes_on_the_same_path() {
    // Arrange / Act: 같은 스캔, 같은 경로 — 해시가 4분 동안 바이트를 알리지 않는다
    let hasher = scan_a_touched_file(false);

    // Assert: 3분 경계를 넘긴 뒤에는 멈춤 (도중에 이미 판정됐다)
    assert_eq!(hasher.reports.load(Ordering::SeqCst), 0);
    assert!(
        hasher.stalled_during.load(Ordering::SeqCst),
        "must be stalled once 3 minutes pass without bytes"
    );
    assert!(hasher.stalled_at_end.load(Ordering::SeqCst));
}
