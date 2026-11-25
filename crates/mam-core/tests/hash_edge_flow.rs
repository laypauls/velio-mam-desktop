//! 중복 검사 단계 판정 통합 시험 (크기 -> 앞뒤 4K -> 전체 해시) — T-hash-edge-05~08, 10, 11, 15b + 백필 규칙.
//!
//! 분류: Business rule (오판 회귀 가드·전체 해시 생략·늦은 충돌) / Happy (완전 복사본) / Concurrency (같은 작업 재실행 멱등) /
//! External (해시 실패 재시도) / Boundary (백필 대상 판정)
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기(스레드). 해시 계산기는 호출 횟수를 세는 대역을 주입한다.

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::env::Env;
use common::proc_env::{ProcEnv, no_tools};
use mam_core::query::asset_detail;
use mam_core::scan::{ScanOutcome, scan_folder};
use mam_core::{Blake3Hasher, ContentHasher, CoreError, EdgeFile, HashedFile, VolumeKind};

/// 호출 횟수를 세고, 필요하면 전체 해시를 실패시키는 해시 계산기.
#[derive(Default)]
struct Probe {
    full: AtomicUsize,
    edge: AtomicUsize,
    fail_full: AtomicBool,
}

impl ContentHasher for Probe {
    fn hash_file(
        &self,
        path: &std::path::Path,
        cancel: &AtomicBool,
    ) -> Result<HashedFile, CoreError> {
        self.full.fetch_add(1, Ordering::SeqCst);
        if self.fail_full.load(Ordering::SeqCst) {
            return Err(CoreError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("sharing violation"),
            });
        }
        Blake3Hasher.hash_file(path, cancel)
    }
    fn edge_signature(
        &self,
        path: &std::path::Path,
        cancel: &AtomicBool,
    ) -> Result<EdgeFile, CoreError> {
        self.edge.fetch_add(1, Ordering::SeqCst);
        mam_core::edge_signature(path, cancel)
    }
}

fn setup() -> (ProcEnv, Arc<Probe>) {
    let probe = Arc::new(Probe::default());
    let p = ProcEnv::with_hasher(Env::new(), no_tools(), probe.clone());
    (p, probe)
}

/// 20,000 바이트. 앞 4K·뒤 4K 는 `seed` 로, 가운데 한 바이트는 `mid` 로 정해진다.
fn media(seed: u8, mid: u8) -> Vec<u8> {
    let mut b: Vec<u8> = (0..20_000).map(|i| (i % 251) as u8 ^ seed).collect();
    b[10_000] = mid;
    b
}

fn add_and_drain(p: &ProcEnv, rel: &str) -> i64 {
    let folder = p
        .env
        .ingest
        .add_folder(p.env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id;
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle(), "jobs did not drain:\n{}", p.jobs_dump());
    folder
}

fn rescan_and_drain(p: &ProcEnv, folder: i64) {
    let _quiet = p.env.ingest.suppress_watch(&p.env.path("lib"));
    p.env.clock.advance(60_000);
    match scan_folder(
        p.env.ingest.env(),
        folder,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        ScanOutcome::Completed { .. } => {}
        other => panic!("expected Completed, got {other:?}"),
    }
    p.env.ingest.jobs().notify();
    assert!(p.wait_idle(), "jobs did not drain:\n{}", p.jobs_dump());
}

fn hash_of(p: &ProcEnv, name: &str) -> Option<Vec<u8>> {
    let id = p.asset_of(name);
    p.env.read(|c| {
        c.query_row(
            "SELECT content_hash FROM media_assets WHERE asset_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn edge_set(p: &ProcEnv, name: &str) -> bool {
    let id = p.asset_of(name);
    p.env.count(&format!(
        "SELECT COUNT(*) FROM media_assets WHERE asset_id = {id} AND edge_sig IS NOT NULL"
    )) == 1
}

#[test]
fn t_hash_edge_07_unique_size_stores_only_the_edge_sig_and_skips_the_full_hash() {
    // Arrange
    let (p, probe) = setup();
    p.put("lib/a.jpg", &media(1, 0));

    // Act
    add_and_drain(&p, "lib");
    p.shutdown();

    // Assert: 앞뒤 체크섬만 읽었고 전체 해시는 계산하지 않았다 (pending 유지 = "검사 완료, 전체 해시 불필요")
    assert!(edge_set(&p, "a.jpg"));
    assert_eq!(hash_of(&p, "a.jpg"), None);
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'pending'"),
        1
    );
    assert_eq!(
        (
            probe.edge.load(Ordering::SeqCst),
            probe.full.load(Ordering::SeqCst)
        ),
        (1, 0)
    );
    assert_eq!(
        p.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash'"),
        0
    );
}

#[test]
fn t_hash_edge_05_same_size_and_edges_with_a_different_middle_are_not_duplicates() {
    // Arrange: 크기·앞 4K·뒤 4K 가 같고 가운데 1바이트만 다른 두 파일 (오판 회귀 가드)
    let (p, probe) = setup();
    p.put("lib/a.jpg", &media(1, 0));
    p.put("lib/b.jpg", &media(1, 99));
    let before = common::env::snapshot(&p.env.path("lib"));

    // Act
    add_and_drain(&p, "lib");
    p.shutdown();

    // Assert: 앞뒤가 같아 전체 해시를 계산했지만 해시가 다르다 -> 중복본이 아니고 병합도 없다
    let (a, b) = (hash_of(&p, "a.jpg").unwrap(), hash_of(&p, "b.jpg").unwrap());
    assert_ne!(a, b);
    assert_eq!(probe.full.load(Ordering::SeqCst), 2);
    let (ida, idb) = (p.asset_of("a.jpg"), p.asset_of("b.jpg"));
    for id in [ida, idb] {
        assert!(
            p.env
                .read(|c| asset_detail(c, id).unwrap())
                .duplicates
                .is_empty()
        );
    }
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE status = 'active'"),
        2
    );
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL AND presence = 'present'"
        ),
        2
    );
    assert_eq!(
        before,
        common::env::snapshot(&p.env.path("lib")),
        "originals untouched"
    );
}

#[test]
fn t_hash_edge_06_exact_copies_get_equal_full_hashes_and_show_as_duplicates() {
    // Arrange
    let (p, _probe) = setup();
    p.put("lib/a.jpg", &media(2, 7));
    p.put("lib/copy.jpg", &media(2, 7));

    // Act
    add_and_drain(&p, "lib");
    p.shutdown();

    // Assert
    assert_eq!(hash_of(&p, "a.jpg"), hash_of(&p, "copy.jpg"));
    assert!(hash_of(&p, "a.jpg").is_some());
    let (ida, idc) = (p.asset_of("a.jpg"), p.asset_of("copy.jpg"));
    let dup: Vec<i64> = p
        .env
        .read(|c| asset_detail(c, ida).unwrap())
        .duplicates
        .iter()
        .map(|s| s.asset_id)
        .collect();
    assert_eq!(dup, vec![idc]);
}

#[test]
fn t_hash_edge_08a_late_arrival_with_the_same_edge_makes_both_assets_hash_fully() {
    // Arrange: 크기가 유일해 전체 해시를 생략한 자산이 먼저 있다
    let (p, probe) = setup();
    p.put("lib/a.jpg", &media(3, 1));
    let folder = add_and_drain(&p, "lib");
    assert_eq!(hash_of(&p, "a.jpg"), None);
    assert_eq!(probe.full.load(Ordering::SeqCst), 0);

    // Act: 같은 크기·같은 앞뒤의 복사본이 나중에 들어온다
    p.put("lib/late.jpg", &media(3, 1));
    rescan_and_drain(&p, folder);
    p.shutdown();

    // Assert: 상대 파일을 다시 읽지 않고도(edge 는 저장돼 있다) 양쪽 모두 전체 해시를 얻는다
    assert!(hash_of(&p, "a.jpg").is_some());
    assert_eq!(hash_of(&p, "a.jpg"), hash_of(&p, "late.jpg"));
    assert_eq!(
        probe.edge.load(Ordering::SeqCst),
        2,
        "each file's edge is read once"
    );
    assert_eq!(probe.full.load(Ordering::SeqCst), 2);
}

#[test]
fn t_hash_edge_08b_late_arrival_with_a_different_edge_leaves_both_without_a_full_hash() {
    // Arrange
    let (p, probe) = setup();
    p.put("lib/a.jpg", &media(3, 1));
    let folder = add_and_drain(&p, "lib");

    // Act: 같은 크기지만 앞 4K 가 다르다
    p.put("lib/other.jpg", &media(4, 1));
    rescan_and_drain(&p, folder);
    p.shutdown();

    // Assert
    assert_eq!(hash_of(&p, "a.jpg"), None);
    assert_eq!(hash_of(&p, "other.jpg"), None);
    assert!(edge_set(&p, "a.jpg") && edge_set(&p, "other.jpg"));
    assert_eq!(probe.full.load(Ordering::SeqCst), 0);
}

#[test]
fn t_hash_edge_10_rerunning_hash_jobs_changes_nothing() {
    // Arrange: 충돌 쌍 + 유일 자산을 끝까지 처리
    let (p, probe) = setup();
    p.put("lib/a.jpg", &media(5, 1));
    p.put("lib/copy.jpg", &media(5, 1));
    p.put("lib/solo.jpg", &vec![9u8; 5_000]);
    add_and_drain(&p, "lib");
    let state = |p: &ProcEnv| {
        (
            p.env.count("SELECT SUM(revision) FROM media_assets"),
            p.env.count("SELECT COUNT(*) FROM search_outbox"),
            p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE content_hash IS NOT NULL"),
            p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL"),
            probe.edge.load(Ordering::SeqCst),
            probe.full.load(Ordering::SeqCst),
        )
    };
    let before = state(&p);
    assert_eq!(before.2, 2);

    // Act: 모든 자산에 hash 작업을 다시 등록해 실행
    let now = p.env.clock_now();
    p.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO jobs (kind, asset_id, priority, created_at, updated_at)
                 SELECT 'hash', asset_id, 5, ?1, ?1 FROM media_assets",
                [now],
            )?;
            Ok(())
        })
        .unwrap();
    p.env.ingest.jobs().notify();
    assert!(p.wait_idle());
    p.shutdown();

    // Assert
    assert_eq!(state(&p), before);
}

#[test]
fn t_hash_edge_11_network_volume_asset_with_a_unique_size_gets_no_full_hash() {
    // Arrange: 네트워크 볼륨 (D-1: 볼륨 종류와 무관하게 같은 크기·같은 edge_sig 자산만 전체 해시 대상)
    let (p, probe) = setup();
    let net = p.env.vol.parent().unwrap().join("vol_n");
    fs::create_dir_all(net.join("lib")).unwrap();
    p.env.volumes.add("vol-n", VolumeKind::Network, &net);
    fs::write(net.join("lib").join("n.jpg"), media(6, 1)).unwrap();

    // Act
    p.env
        .ingest
        .add_folder(net.join("lib").to_str().unwrap())
        .unwrap();
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());
    p.shutdown();

    // Assert
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM volumes WHERE kind = 'network'"),
        1
    );
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL AND content_hash IS NULL"
        ),
        1
    );
    assert_eq!(probe.full.load(Ordering::SeqCst), 0);
    assert_eq!(
        p.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash'"),
        0
    );
}

#[test]
fn t_hash_edge_10b_full_hash_failure_is_retried_then_recorded_as_failed() {
    // Arrange: 충돌 쌍 + 전체 해시가 항상 실패하는 환경 (앞뒤 체크섬은 정상)
    let (p, probe) = setup();
    probe.fail_full.store(true, Ordering::SeqCst);
    p.put("lib/a.jpg", &media(7, 1));
    p.put("lib/b.jpg", &media(7, 1));

    // Act
    add_and_drain(&p, "lib");
    p.shutdown();

    // Assert: 재시도 소진 뒤 둘 다 failed, 해시는 기록되지 않았다 (수동 재시도 대상)
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM media_assets WHERE hash_state = 'failed' AND content_hash IS NULL"
        ),
        2
    );
    assert_eq!(
        p.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash'"),
        0
    );
}

#[test]
fn t_hash_edge_15b_backfill_fills_edge_sig_once_and_does_not_requeue_assets_that_need_no_full_hash()
{
    // Arrange: 처리 기능이 없던 시절에 스캔한 자산 (edge_sig NULL = V11 이전과 같은 상태)
    let env = Env::new();
    for (i, name) in ["a.jpg", "b.jpg", "c.jpg"].iter().enumerate() {
        let path = env.path(&format!("lib/{name}"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![i as u8 + 1; 3_000 + i * 100]).unwrap();
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap();
    let probe = Arc::new(Probe::default());
    let p = ProcEnv::with_hasher(env, no_tools(), probe.clone());
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NULL"),
        3
    );

    // Act: 보충 등록 -> 실행
    let report = p.env.ingest.supervise_once().unwrap().backfill.unwrap();
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());
    let clock = Arc::clone(&p.env.clock);
    let again =
        mam_core::processing::backfill_jobs(&p.env.pool, move || mam_db::Clock::now_ms(&*clock))
            .unwrap();
    p.shutdown();

    // Assert: edge_sig 가 채워졌고, 전체 해시는 없고(크기 유일), 다시 등록되지 않는다
    assert_eq!(report.hash, 3);
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL"),
        3
    );
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE content_hash IS NOT NULL"),
        0
    );
    assert_eq!(again.hash, 0);
    assert_eq!(probe.full.load(Ordering::SeqCst), 0);
}

#[test]
fn t_hash_edge_15c_backfill_selects_only_assets_that_still_need_a_hash_job() {
    // Arrange: 상태별 자산을 직접 심는다 (E(n) = 서로 다른 32바이트 값)
    let env = Env::new();
    let e = |n: u32| format!("CAST(printf('%032d', {n}) AS BLOB)");
    let rows: Vec<(i64, &str, i64, String)> = vec![
        // (id, 상태/해시, 크기, edge_sig)
        (1, "pending", 100, "NULL".into()),    // edge 없음 -> 등록
        (2, "pending", 100, e(1)),             // 크기 100·sig 1, 상대 없음 -> 제외
        (3, "pending", 200, e(2)),             // 같은 크기·같은 sig 쌍 -> 등록
        (4, "pending", 200, e(2)),             // 쌍 -> 등록
        (5, "failed", 100, "NULL".into()),     // failed 는 건드리지 않는다
        (6, "done", 100, "NULL".into()),       // 해시는 있고 edge 없음 -> 등록
        (7, "done", 100, e(3)),                // 해시·edge 모두 있음 -> 제외
        (8, "pending", 100, e(4)),             // 2 와 크기 같고 sig 다름 -> 제외
        (9, "superseded", 100, "NULL".into()), // 비활성 -> 제외
        (10, "pending", 300, e(5)),            // 상대(11)가 deleted -> 제외
        (11, "deleted", 300, e(5)),
    ];
    env.pool
        .write(mam_db::Priority::High, move |c| {
            for (id, kind, size, edge) in &rows {
                let (status, state, hash) = match *kind {
                    "done" => ("active", "done", "zeroblob(32)"),
                    "superseded" | "deleted" => ("active", "pending", "NULL"),
                    other => ("active", other, "NULL"),
                };
                let status = if matches!(*kind, "superseded" | "deleted") { *kind } else { status };
                c.execute_batch(&format!(
                    "INSERT INTO media_assets (asset_id, media_type, status, content_hash, hash_algo, hash_state, file_size, modified_at, created_at, updated_at, edge_sig)
                     VALUES ({id}, 'image', '{status}', {hash}, {algo}, '{state}', {size}, 0, 0, 0, {edge})",
                    algo = if hash == "NULL" { "NULL" } else { "'blake3'" },
                ))?;
            }
            Ok(())
        })
        .unwrap();

    // Act
    let clock = Arc::clone(&env.clock);
    let report =
        mam_core::processing::backfill_jobs(&env.pool, move || mam_db::Clock::now_ms(&*clock))
            .unwrap();

    // Assert
    let queued: Vec<i64> = env.read(|c| {
        let mut stmt = c
            .prepare("SELECT asset_id FROM jobs WHERE kind = 'hash' ORDER BY asset_id")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    assert_eq!(queued, vec![1, 3, 4, 6]);
    assert_eq!(report.hash, 4);
}
