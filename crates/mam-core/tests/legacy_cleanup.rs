//! 기존 보관 데이터 1회 정리 (T-legacy-01 ~ 06).
//!
//! 분류: Happy (01 대상만 정리) / Business rule (01 보존 조건, 02 키셋 경계) / Boundary (03 후보 없음 -> 백업 없이 완료 표시) /
//! External (04 백업 실패) / Concurrency-재개 (05 중단 뒤 재실행, 06 반복 실행). 실제 SQLite.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};

use common::env::{Env, snapshot};
use common::purge::{exec, insert_assets, seed_linked, status_counts, total_linked, volume_id};
use mam_core::ingest::LegacyCleanupReport;
use mam_core::scan::IngestError;

fn add(env: &Env, rel: &str) -> i64 {
    fs::create_dir_all(env.path(rel)).unwrap();
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn hook(env: &Env, fail: bool) -> Arc<Mutex<u32>> {
    let calls = Arc::new(Mutex::new(0));
    let c = Arc::clone(&calls);
    env.ingest.set_backup_hook(Arc::new(move || {
        *c.lock().unwrap() += 1;
        if fail { Err("disk full".into()) } else { Ok(()) }
    }));
    calls
}

/// 자산 하나를 만들고 활성 위치를 detach 한다 (`rel` 에 detach 된 위치만 남긴다).
fn detached_asset(env: &Env, rel: &str, status: &str) -> i64 {
    let id = insert_assets(env, "vol-a", "tmp", 1, "image")[0];
    let vid = volume_id(env, "vol-a");
    exec(env, &format!(
        "DELETE FROM file_locations WHERE asset_id = {id};
         INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, detached_at, last_seen_at)
           VALUES ({id}, {vid}, '{rel}', '{rel}', 5, 1);
         UPDATE media_assets SET status = '{status}' WHERE asset_id = {id};"
    ));
    id
}

fn done(env: &Env) -> bool {
    env.read(|c| mam_db::link_cleanup::legacy_cleanup_done(c).unwrap())
}

#[test]
fn t_legacy_01_only_active_assets_without_an_active_location_outside_every_folder_are_cleaned() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    let target = detached_asset(&env, "gone/a.jpg", "active"); // 제거한 폴더의 옛 보관 자산
    let moved_out = detached_asset(&env, "lib/b.jpg", "active"); // 관리 폴더 밖으로 이동 (옛 위치는 등록된 폴더 안)
    let live = insert_assets(&env, "vol-a", "lib", 1, "image")[0]; // 활성 위치가 있다
    let deleted = detached_asset(&env, "gone/d.jpg", "deleted"); // 원본 삭제
    let superseded = detached_asset(&env, "gone/e.jpg", "superseded"); // 내용 변경
    let no_location = insert_assets(&env, "vol-a", "tmp", 1, "image")[0];
    exec(&env, &format!("DELETE FROM file_locations WHERE asset_id = {no_location}"));
    let all = [target, moved_out, live, deleted, superseded, no_location];
    seed_linked(&env, &all);
    let before: Vec<i64> = all.iter().map(|id| total_linked(&env, &[*id])).collect();
    let calls = hook(&env, false);
    let originals = snapshot(&env.vol);

    // Act
    let report = env.ingest.run_legacy_cleanup(&|| false).unwrap();

    // Assert
    assert_eq!(report, LegacyCleanupReport { purged: 1, already_done: false, interrupted: false });
    assert_eq!(total_linked(&env, &[target]), 0);
    assert_eq!(status_counts(&env, &[target], "deleted"), 1);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE asset_id = {target}")), 0);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete' AND asset_id = {target}")), 1);
    for (i, id) in all.iter().enumerate().skip(1) {
        assert_eq!(total_linked(&env, &[*id]), before[i], "asset #{i} must be untouched");
    }
    assert_eq!(status_counts(&env, &[moved_out, live], "active"), 2);
    assert_eq!(status_counts(&env, &[deleted], "deleted") + status_counts(&env, &[superseded], "superseded"), 2);
    assert_eq!(*calls.lock().unwrap(), 1, "one backup before the first deletion");
    assert!(done(&env));
    assert_eq!(snapshot(&env.vol), originals);
}

#[test]
fn t_legacy_02_candidates_that_are_kept_do_not_hide_later_targets_across_batches() {
    // Arrange: 앞쪽 501 건은 보존 대상(이동된 자산), 그 뒤에 정리 대상 3건 + 1203건 후보 경계
    let env = Env::new();
    add(&env, "lib");
    let vid = volume_id(&env, "vol-a");
    let kept = insert_assets(&env, "vol-a", "lib", 501, "image");
    exec(&env, &format!(
        "UPDATE file_locations SET detached_at = 5 WHERE asset_id IN (SELECT asset_id FROM media_assets WHERE asset_id <= {})",
        kept.last().unwrap()
    ));
    let targets = insert_assets(&env, "vol-a", "gone", 3, "image");
    exec(&env, &format!("UPDATE file_locations SET detached_at = 5 WHERE volume_id = {vid} AND rel_path LIKE 'gone/%'"));
    seed_linked(&env, &targets);
    hook(&env, false);

    // Act
    let report = env.ingest.run_legacy_cleanup(&|| false).unwrap();

    // Assert
    assert_eq!(report.purged, 3);
    assert_eq!(status_counts(&env, &targets, "deleted"), 3);
    assert_eq!(status_counts(&env, &kept, "active"), 501);
}

#[test]
fn t_legacy_03_without_candidates_it_marks_done_without_a_backup() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    insert_assets(&env, "vol-a", "lib", 2, "image");
    let calls = hook(&env, false);

    // Act
    let report = env.ingest.run_legacy_cleanup(&|| false).unwrap();

    // Assert
    assert_eq!(report.purged, 0);
    assert!(done(&env));
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn t_legacy_04_failed_backup_deletes_nothing_and_leaves_the_flag_unset_for_the_next_start() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    let target = detached_asset(&env, "gone/a.jpg", "active");
    seed_linked(&env, &[target]);
    let before = total_linked(&env, &[target]);
    hook(&env, true);

    // Act
    let err = env.ingest.run_legacy_cleanup(&|| false).unwrap_err();

    // Assert
    assert!(matches!(err, IngestError::BackupFailed), "{err:?}");
    assert!(!done(&env));
    assert_eq!(total_linked(&env, &[target]), before);
    assert_eq!(status_counts(&env, &[target], "active"), 1);
    // 다음 시작: 백업이 되면 끝난다
    hook(&env, false);
    assert_eq!(env.ingest.run_legacy_cleanup(&|| false).unwrap().purged, 1);
    assert!(done(&env));
}

#[test]
fn t_legacy_05_interrupted_cleanup_has_no_flag_and_a_rerun_finishes_it() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    let targets: Vec<i64> = (0..3).map(|i| detached_asset(&env, &format!("gone/{i}.jpg"), "active")).collect();
    seed_linked(&env, &targets);
    hook(&env, false);

    // Act 1: 첫 배치 전에 중단
    let first = env.ingest.run_legacy_cleanup(&|| true).unwrap();

    // Assert 1
    assert!(first.interrupted && first.purged == 0);
    assert!(!done(&env));
    assert_eq!(status_counts(&env, &targets, "active"), 3);

    // Act 2
    let second = env.ingest.run_legacy_cleanup(&|| false).unwrap();

    // Assert 2
    assert_eq!(second.purged, 3);
    assert_eq!(total_linked(&env, &targets), 0);
}

#[test]
fn t_legacy_06_second_run_does_nothing_even_if_new_candidates_appear() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    let first_target = detached_asset(&env, "gone/a.jpg", "active");
    let calls = hook(&env, false);
    env.ingest.run_legacy_cleanup(&|| false).unwrap();
    let later = detached_asset(&env, "gone/b.jpg", "active"); // 이후에 생긴 것은 이 1회 정리의 대상이 아니다
    seed_linked(&env, &[later]);

    // Act
    let report = env.ingest.run_legacy_cleanup(&|| false).unwrap();

    // Assert
    assert!(report.already_done);
    assert_eq!(status_counts(&env, &[first_target], "deleted"), 1);
    assert_eq!(status_counts(&env, &[later], "active"), 1);
    assert_eq!(*calls.lock().unwrap(), 1);
}
