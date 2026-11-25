//! 폴더 이전(D-6) 통합 시험 — 검증·동시성·원자성 (T-relocate-04, 05, 06, 08, 13, 14[측정, ignore]).
//!
//! 분류: Validation (04) / Concurrency (05, 08) / Business rule (06 원자성·revision, 13 작업·감시) / Boundary (14 규모).
//! 실제 SQLite + 임시 디렉터리 볼륨 둘. 거부된 요청은 DB 를 한 바이트도 바꾸지 않아야 한다.

mod common;

use std::cell::Cell;
use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use common::env::{snapshot, wait_until};
use common::reloc::Reloc;
use mam_core::jobs::JobKind;
use mam_core::scan::{IngestError, ScanOutcome, scan_folder};

#[test]
fn t_relocate_04_invalid_targets_are_rejected_without_any_db_change() {
    // Arrange: lib(대상 폴더) 와 other(다른 관리 폴더), 파일 하나
    let r = Reloc::new();
    let id = r.seeded();
    r.env.write("other/o.jpg", 9);
    fs::create_dir_all(r.env.path("other/sub")).unwrap();
    let other = r.add("other");
    fs::create_dir_all(r.path_b("empty")).unwrap();
    let file = r.env.path("lib/a.jpg");
    let base = r.env.vol.parent().unwrap().to_path_buf();
    let fixtures = snapshot(&r.env.vol);
    let before = r.db_state();
    type Check = fn(&IngestError) -> bool;
    let cases: Vec<(&str, String, Check)> = vec![
        ("same path", r.env.path("lib").to_str().unwrap().into(), |e| matches!(e, IngestError::SamePath)),
        ("nonexistent", r.path_b("nope").to_str().unwrap().into(), |e| matches!(e, IngestError::InvalidInput(_))),
        ("a file", file.to_str().unwrap().into(), |e| matches!(e, IngestError::InvalidInput(_))),
        ("relative", "lib".into(), |e| matches!(e, IngestError::InvalidInput(_))),
        ("empty", String::new(), |e| matches!(e, IngestError::InvalidInput(_))),
        ("other folder itself", r.env.path("other").to_str().unwrap().into(), |e| matches!(e, IngestError::AlreadyRegistered)),
        ("child of other", r.env.path("other").join("sub").to_str().unwrap().into(), |e| {
            matches!(e, IngestError::NestedFolder { inside_existing: true })
        }),
        ("own child", r.env.path("lib/sub").to_str().unwrap().into(), |e| {
            matches!(e, IngestError::NestedFolder { inside_existing: true })
        }),
        ("own parent (volume root)", r.env.vol.to_str().unwrap().into(), |e| {
            matches!(e, IngestError::NestedFolder { inside_existing: false })
        }),
        ("MAMData itself", r.env.data.to_str().unwrap().into(), |e| {
            matches!(e, IngestError::OverlapsManagedData { contains: false })
        }),
        ("parent of MAMData", base.to_str().unwrap().into(), |e| {
            matches!(e, IngestError::OverlapsManagedData { contains: true })
        }),
    ];

    for (name, target, is_expected) in cases {
        // Act
        let err = r.env.ingest.relocate_folder(id, &target).expect_err(name);
        // Assert
        assert!(is_expected(&err), "{name}: {err:?}");
        assert_eq!(before, r.db_state(), "{name}: DB changed");
    }
    // 대상이 두 번째 폴더의 부모: vol-a 루트는 위 "own parent" 가 이미 덮는다 — 다른 폴더 기준으로도 확인
    let err = r.env.ingest.relocate_folder(other, r.env.vol.to_str().unwrap()).unwrap_err();
    assert!(matches!(err, IngestError::NestedFolder { inside_existing: false }), "{err:?}");
    // 없는 폴더·비활성 폴더
    assert!(matches!(r.relocate(9_999, &r.path_b("empty")), Err(IngestError::NotFound)));
    r.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute("UPDATE managed_folders SET enabled = 0 WHERE folder_id = ?1", [other])?;
            Ok(())
        })
        .unwrap();
    let disabled = r.db_state();
    assert!(matches!(r.relocate(other, &r.path_b("empty")), Err(IngestError::NotFound)));
    assert_eq!(disabled, r.db_state());
    assert_eq!(fixtures, snapshot(&r.env.vol), "originals untouched");
}

#[test]
fn t_relocate_05_running_scan_blocks_relocation_but_pending_and_paused_do_not() {
    // Arrange: 등록 직후 스캔 작업이 대기 중이다
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    let job = r.env.ingest.jobs().claim_next(&[JobKind::Scan]).unwrap().expect("pending scan job");
    assert_eq!(job.kind, JobKind::Scan);
    let before = r.db_state();

    // Act + Assert: 실행 중이면 거부하고 아무것도 바꾸지 않는다
    let err = r.relocate(id, &r.path_b("lib")).unwrap_err();
    assert!(matches!(err, IngestError::ScanRunning), "{err:?}");
    assert_eq!(before, r.db_state());

    // 실행 중 상태를 대기로 되돌리고 일시정지하면 이전이 된다
    r.env.ingest.jobs().pause(Some(JobKind::Scan)).unwrap();
    r.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE jobs SET status = 'pending' WHERE kind = 'scan'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(r.relocate(id, &r.path_b("lib")).unwrap().relocated, 3);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan'"), 1);
}

#[test]
fn t_relocate_06_each_asset_gets_one_revision_bump_and_one_event_and_failure_rolls_everything_back() {
    // Arrange
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    let rev = |r: &Reloc| r.env.count("SELECT SUM(revision) FROM media_assets");
    let (rev0, events0) = (rev(&r), r.env.count("SELECT COUNT(*) FROM search_outbox"));

    // Act 1: 중간 행에서 실패를 주입한다 (마지막 위치 갱신 시 트리거가 중단)
    r.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute_batch(
                "CREATE TRIGGER boom BEFORE UPDATE OF path_key ON file_locations
                 WHEN NEW.rel_path LIKE '%c.png' BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )?;
            Ok(())
        })
        .unwrap();
    let before = r.db_state();
    let failed = r.relocate(id, &r.path_b("lib"));

    // Assert 1: 폴더·위치·revision·색인 이벤트·작업 모두 그대로
    assert!(failed.is_err());
    assert_eq!(before, r.db_state());

    // Act 2: 트리거 제거 후 다시
    r.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute_batch("DROP TRIGGER boom")?;
            Ok(())
        })
        .unwrap();
    r.relocate(id, &r.path_b("lib")).unwrap();

    // Assert 2: 자산마다 revision +1, upsert 이벤트 +1
    assert_eq!(rev(&r), rev0 + 3);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM search_outbox"), events0 + 3);
    assert_eq!(
        r.env.count(
            "SELECT COUNT(*) FROM media_assets a WHERE EXISTS (SELECT 1 FROM search_outbox o
             WHERE o.asset_id = a.asset_id AND o.revision = a.revision AND o.event_type = 'upsert')"
        ),
        3,
        "the latest event of every asset carries its current revision"
    );
}

#[test]
fn t_relocate_08_a_scan_that_started_before_the_relocation_writes_nothing_afterwards() {
    // Arrange: 스캔이 첫 디렉터리를 보기 직전에 (옛 폴더 정보를 들고) 이전이 끝난다. 옛 위치에는 새 파일이 있다
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    r.env.write("lib/late.jpg", 7);
    let old_volume = r.volume_id("vol-a");
    let done = Cell::new(false);
    let stop = || {
        if !done.replace(true) {
            r.relocate(id, &r.path_b("lib")).unwrap();
        }
        false
    };

    // Act
    r.env.clock.advance(60_000);
    let outcome = scan_folder(r.env.ingest.env(), id, &stop, &AtomicBool::new(false)).unwrap();

    // Assert: 폴더가 바뀌었으므로 중단(Gone)하고 옛 볼륨에는 아무 위치도 생기지 않는다
    assert_eq!(outcome, ScanOutcome::Gone);
    assert_eq!(
        r.env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE volume_id = {old_volume} AND detached_at IS NULL")),
        0
    );
    assert_eq!(r.env.count("SELECT COUNT(*) FROM file_locations WHERE rel_path LIKE '%late.jpg'"), 0);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM media_assets"), 3);
}

#[test]
fn t_relocate_13_enqueues_exactly_one_pending_scan_and_restarts_the_watcher_on_the_new_root() {
    // Arrange: 실패한 스캔 작업과 백오프 중인 대기 작업이 따로 있는 폴더, 새 경로로 복사본이 있다
    let r = Reloc::new();
    let id = r.seeded();
    r.copy_tree(&r.env.path("lib"), &r.path_b("lib"));
    r.env.ingest.jobs().pause(Some(JobKind::Scan)).unwrap(); // 이전 후 스캔은 멈춘 채 감시만 본다
    r.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO jobs (kind, folder_id, priority, status, attempts, not_before, created_at, updated_at)
                 VALUES ('scan', ?1, 10, 'failed', 3, 0, 0, 0)",
                [id],
            )?;
            c.execute(
                "UPDATE jobs SET not_before = 99999999999999, attempts = 2 WHERE kind = 'scan' AND status = 'pending'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    r.env.ingest.start().unwrap();

    // Act
    r.relocate(id, &r.path_b("lib")).unwrap();

    // Assert 1: 실패 행은 지워지고, 대기 작업은 하나뿐이며 즉시 실행 가능하다
    assert_eq!(r.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'failed'"), 0);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending'"), 1);
    assert_eq!(r.env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND status = 'pending' AND not_before = 0 AND attempts = 0"), 1);

    // Assert 2: 새 루트에 파일을 만들면 감시가 반영한다 (스캔은 일시정지 상태이므로 감시만이 이 자산을 만들 수 있다)
    fs::write(r.path_b("lib/watched.jpg"), vec![5u8; 80]).unwrap();
    // 디바운스는 주입 시계로 판단한다 — 폴링하는 동안 시계를 전진시킨다
    let seen = wait_until(Duration::from_secs(30), || {
        r.env.clock.advance(100);
        r.env.count("SELECT COUNT(*) FROM file_locations WHERE rel_path LIKE '%watched.jpg'") == 1
    });
    r.env.ingest.shutdown();
    assert!(seen, "watcher must observe the new root");
}

/// 측정 전용 (`cargo test -p mam-core --test folder_relocate_guard -- --ignored --nocapture`, `RELOCATE_N=100000`).
/// 합성 위치 N 개를 옮기는 트랜잭션 시간(왕복 반복)과 WAL 크기를 출력한다. 합성 메타데이터이므로 실제 스캔 성능은 아니다.
#[test]
#[ignore = "measurement"]
fn t_relocate_14_measure_transaction_time_by_location_count() {
    // Arrange
    let n: usize = std::env::var("RELOCATE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(10_000);
    let rounds: usize = std::env::var("RELOCATE_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    let r = Reloc::new();
    r.env.write("lib/seed.jpg", 1);
    let id = r.add("lib");
    fs::create_dir_all(r.path_b("lib")).unwrap();
    let vol_a = r.volume_id("vol-a");
    r.env
        .pool
        .write(mam_db::Priority::High, move |c| {
            c.execute_batch(&format!(
                "WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < {n})
                 INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                 SELECT 'image', i, 0, 0, 0 FROM s;
                 WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < {n})
                 INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                 SELECT i, {vol_a}, 'lib\\d' || (i % 100) || '\\f' || i || '.jpg',
                        'lib/d' || (i % 100) || '/f' || i || '.jpg', 0 FROM s;"
            ))?;
            Ok(())
        })
        .unwrap();

    // Act: lib(vol-a) <-> lib(vol-b) 왕복
    let mut ms = Vec::new();
    for round in 0..rounds {
        let target = if round % 2 == 0 { r.path_b("lib") } else { r.env.path("lib") };
        let t = Instant::now();
        let out = r.relocate(id, &target).unwrap();
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(out.relocated as usize, n);
        r.env.clock.advance(1_000);
        r.env.pool.write(mam_db::Priority::High, |c| {
            c.execute("DELETE FROM jobs WHERE kind = 'scan'", [])?;
            Ok(())
        }).unwrap();
    }

    // Assert + report
    ms.sort_by(f64::total_cmp);
    let q = |p: f64| ms[((ms.len() as f64 - 1.0) * p).round() as usize];
    let wal = fs::metadata(r.env.data.join("mam.sqlite-wal")).map(|m| m.len()).unwrap_or(0);
    eprintln!(
        "RELOCATE locations={} rounds={rounds} ms P50={:.0} P95={:.0} P99={:.0} max={:.0} wal_bytes={wal}",
        n,
        q(0.5),
        q(0.95),
        q(0.99),
        ms[ms.len() - 1]
    );
}
