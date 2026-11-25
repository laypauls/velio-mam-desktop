//! 관리 폴더 제거 → 연결 데이터 삭제 (T-rmv-01 ~ 12).
//!
//! 분류: Happy (01 전수 삭제·보존 항목, 09 재추가) / Boundary (02 배치 경계, 08 반복 실행) / Concurrency-재개 (03) /
//! Business rule (04 같은 디스크 다른 폴더, 05 경로 변경으로 옮겨진 자산, 06 컬렉션·태그 이름, 11 busy) /
//! External (07 오프라인, 10 백업 실패). 실제 SQLite + 임시 디렉터리 볼륨. 원본 보호: 내용·수정 시각 스냅샷 비교.

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use common::env::{Env, snapshot};
use common::purge::{
    exec, global_plugin_kv_rows, ids_sql, insert_assets, linked_rows, seed_linked, status_counts,
    total_linked,
};
use mam_core::ingest::{FolderRemovalProgress, RemovalRun};
use mam_core::scan::{IngestError, ScanOutcome, scan_folder};

fn add(env: &Env, rel: &str) -> i64 {
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn scan(env: &Env, id: i64) {
    let o = scan_folder(env.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    assert!(matches!(o, ScanOutcome::Completed { .. }), "{o:?}");
}

/// `path_key` 가 `prefix/` 로 시작하는 위치의 자산 id (활성·detach 모두)
fn ids_under(env: &Env, prefix: &str) -> Vec<i64> {
    env.read(|c| {
        let mut stmt = c
            .prepare("SELECT DISTINCT asset_id FROM file_locations WHERE path_key LIKE ?1 ORDER BY asset_id")
            .unwrap();
        stmt.query_map([format!("{prefix}/%")], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    })
}

fn events(env: &Env) -> Arc<Mutex<Vec<FolderRemovalProgress>>> {
    let log: Arc<Mutex<Vec<FolderRemovalProgress>>> = Arc::default();
    let sink = Arc::clone(&log);
    env.ingest
        .set_removal_listener(Arc::new(move |p| sink.lock().unwrap().push(p)));
    log
}

/// 호출 횟수를 세는 백업 훅 (`fail` 이면 실패)
fn backup_hook(env: &Env, fail: bool) -> Arc<Mutex<u32>> {
    let calls = Arc::new(Mutex::new(0));
    let c = Arc::clone(&calls);
    env.ingest.set_backup_hook(Arc::new(move || {
        *c.lock().unwrap() += 1;
        if fail { Err("disk full".to_string()) } else { Ok(()) }
    }));
    calls
}

fn folder_ids(env: &Env) -> Vec<i64> {
    env.ingest.list_folders().unwrap().iter().map(|f| f.folder_id).collect()
}

fn touch_cache(root: &std::path::Path, id: i64) -> Vec<std::path::PathBuf> {
    use mam_media::{ThumbSize, strip_path, thumbnail_path};
    let paths = [
        thumbnail_path(root, id, ThumbSize::Small),
        thumbnail_path(root, id, ThumbSize::Large),
        strip_path(root, id),
    ];
    for p in &paths {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"cache").unwrap();
    }
    paths.to_vec()
}

#[test]
fn t_rmv_01_removal_deletes_every_linked_table_keeps_names_and_history_and_never_touches_originals() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/sub/b.jpg", 2);
    env.write("other/c.jpg", 3);
    let (lib, other) = (add(&env, "lib"), add(&env, "other"));
    scan(&env, lib);
    scan(&env, other);
    let (gone, kept) = (ids_under(&env, "lib"), ids_under(&env, "other"));
    assert_eq!((gone.len(), kept.len()), (2, 1));
    seed_linked(&env, &gone);
    seed_linked(&env, &kept);
    let kept_before = total_linked(&env, &kept);
    let cache = env.data.join("cache");
    env.ingest.set_cache_root(cache.clone());
    let gone_files: Vec<_> = gone.iter().flat_map(|id| touch_cache(&cache, *id)).collect();
    let kept_files: Vec<_> = kept.iter().flat_map(|id| touch_cache(&cache, *id)).collect();
    let log = events(&env);
    let backups = backup_hook(&env, false);
    let originals = snapshot(&env.vol);

    // Act
    env.ingest.remove_folder(lib).unwrap();
    // 반환 직후: 목록·검색(숨김 id)에서 이미 빠져 있다
    assert_eq!(folder_ids(&env), vec![other]);
    assert!(env.read(|c| mam_core::scan::hidden_folder_ids(c).unwrap()).contains(&(lib as u64)));
    env.ingest.wait_removals();

    // Assert: 연결 테이블마다 0행
    for (table, n) in linked_rows(&env, &gone) {
        assert_eq!(n, 0, "{table} still has rows of removed assets");
    }
    assert_eq!(status_counts(&env, &gone, "deleted"), 2, "asset rows stay as deleted");
    assert_eq!(global_plugin_kv_rows(&env), 1, "plugin global KV (asset_id NULL) is not asset-linked");
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE asset_id IN ({})", ids_sql(&gone))), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 1);
    // revision 증가 + delete 이벤트 한 건씩 (같은 트랜잭션 — 이벤트 revision 이 자산 revision 과 같다)
    for id in &gone {
        let (rev, ev_rev, n): (i64, i64, i64) = env.read(|c| {
            c.query_row(
                "SELECT a.revision, MAX(o.revision), COUNT(*) FROM media_assets a
                 JOIN search_outbox o ON o.asset_id = a.asset_id AND o.event_type = 'delete' WHERE a.asset_id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        });
        assert_eq!((rev, ev_rev, n), (2, 2, 1));
    }
    // 지우지 않는 것: 태그 이름·컬렉션·AI 사용량 이력, 다른 폴더의 자산과 연결 데이터
    assert_eq!(env.count("SELECT COUNT(*) FROM tags WHERE name_key = 'seed-tag'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM collections WHERE name_key = 'seed-collection'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM ai_usage"), 3);
    assert_eq!(total_linked(&env, &kept), kept_before);
    assert_eq!(status_counts(&env, &kept, "active"), 1);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete' AND asset_id IN ({})", ids_sql(&kept))), 0);
    // 캐시 파일: 지운 자산의 것만 삭제
    assert!(gone_files.iter().all(|p| !p.exists()));
    assert!(kept_files.iter().all(|p| p.exists()));
    // 백업 1회, 진행 이벤트는 마지막에 finished
    assert_eq!(*backups.lock().unwrap(), 1);
    let last = log.lock().unwrap().last().cloned().unwrap();
    assert_eq!((last.folder_id, last.done, last.total, last.finished, last.error), (lib, 2, 2, true, None));
    assert_eq!(snapshot(&env.vol), originals, "originals untouched (content + mtime)");
}

#[test]
fn t_rmv_02_batches_of_500_report_progress_and_commit_atomically() {
    // Arrange: 1203 건 = 500 + 500 + 203
    let env = Env::new();
    fs::create_dir_all(env.path("lib")).unwrap();
    let lib = add(&env, "lib");
    let ids = insert_assets(&env, "vol-a", "lib", 1203, "image");
    seed_linked(&env, &ids[..10]);
    let log = events(&env);

    // Act
    assert!(env.ingest.begin_folder_removal(lib).unwrap());
    let run = env.ingest.run_folder_removal(lib, &|| false).unwrap();

    // Assert
    assert_eq!(run, RemovalRun::Finished);
    let done: Vec<(u64, bool)> = log.lock().unwrap().iter().map(|p| (p.done, p.finished)).collect();
    assert_eq!(done, [(0, false), (500, false), (1000, false), (1203, false), (1203, true)]);
    assert!(log.lock().unwrap().iter().all(|p| p.total == 1203));
    assert_eq!(status_counts(&env, &ids, "deleted"), 1203);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete'"), 1203);
    assert_eq!(total_linked(&env, &ids), 0);
}

#[test]
fn t_rmv_03_interrupted_removal_resumes_after_restart_without_duplicate_events() {
    // Arrange
    let env = Env::new();
    fs::create_dir_all(env.path("lib")).unwrap();
    let lib = add(&env, "lib");
    let ids = insert_assets(&env, "vol-a", "lib", 1203, "image");
    env.ingest.begin_folder_removal(lib).unwrap();
    let polls = std::cell::Cell::new(0);

    // Act 1: 첫 배치가 끝난 뒤 중단 (종료 신호 흉내)
    let run = env
        .ingest
        .run_folder_removal(lib, &|| {
            polls.set(polls.get() + 1);
            polls.get() > 1
        })
        .unwrap();

    // Assert 1: 500 건만 지워졌고 폴더 행은 제거 중으로 남는다 (화면에는 계속 없음)
    assert_eq!(run, RemovalRun::Interrupted);
    assert_eq!(status_counts(&env, &ids, "deleted"), 500);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders WHERE enabled = 0 AND hidden = 1"), 1);
    assert!(folder_ids(&env).is_empty());

    // Act 2: "다음 실행" — 제거 중인 폴더를 찾아 이어서 끝낸다
    assert_eq!(env.ingest.resume_removals().unwrap(), 1);
    env.ingest.wait_removals();

    // Assert 2
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
    assert_eq!(status_counts(&env, &ids, "deleted"), 1203);
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox WHERE event_type = 'delete'"), 1203);
    assert_eq!(env.ingest.resume_removals().unwrap(), 0, "nothing left to resume");
}

#[test]
fn t_rmv_04_other_folders_on_the_same_disk_and_prefix_siblings_are_untouched() {
    // Arrange: lib 와 lib2 는 같은 접두를 가진 형제다
    let env = Env::new();
    for d in ["lib", "lib2", "other"] {
        fs::create_dir_all(env.path(d)).unwrap();
    }
    let (lib, lib2, other) = (add(&env, "lib"), add(&env, "lib2"), add(&env, "other"));
    let a = insert_assets(&env, "vol-a", "lib", 5, "image");
    let b = insert_assets(&env, "vol-a", "lib2", 5, "image");
    let c = insert_assets(&env, "vol-a", "other", 5, "video");
    for ids in [&a, &b, &c] {
        seed_linked(&env, ids);
    }
    let (b_before, c_before) = (total_linked(&env, &b), total_linked(&env, &c));

    // Act
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();

    // Assert
    assert_eq!(total_linked(&env, &a), 0);
    for (ids, before) in [(&b, b_before), (&c, c_before)] {
        assert_eq!(total_linked(&env, ids), before);
        assert_eq!(status_counts(&env, ids, "active"), 5);
        assert_eq!(env.count(&format!("SELECT COUNT(*) FROM file_locations WHERE detached_at IS NULL AND asset_id IN ({})", ids_sql(ids))), 5);
    }
    assert_eq!(folder_ids(&env), vec![lib2, other]);
}

#[test]
fn t_rmv_05_asset_relocated_to_another_folder_keeps_everything_and_loses_only_the_old_location_row() {
    // Arrange: X 는 b 폴더에 활성 위치, a 폴더에 옛 detach 위치 (경로 변경 뒤 남는 모양). Y 는 a 의 일반 자산
    let env = Env::new();
    for d in ["a", "b"] {
        fs::create_dir_all(env.path(d)).unwrap();
    }
    let (a, _b) = (add(&env, "a"), add(&env, "b"));
    let x = insert_assets(&env, "vol-a", "b", 1, "image");
    let y = insert_assets(&env, "vol-a", "a", 1, "image");
    let vid = common::purge::volume_id(&env, "vol-a");
    exec(&env, &format!(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, detached_at, last_seen_at)
         VALUES ({}, {vid}, 'a/old.jpg', 'a/old.jpg', 5, 1)", x[0]));
    seed_linked(&env, &x);
    seed_linked(&env, &y);
    let x_before = total_linked(&env, &x);
    let x_revision = env.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {}", x[0]));

    // Act
    env.ingest.remove_folder(a).unwrap();
    env.ingest.wait_removals();

    // Assert
    assert_eq!(status_counts(&env, &x, "active"), 1);
    assert_eq!(total_linked(&env, &x), x_before);
    assert_eq!(env.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {}", x[0])), x_revision);
    let rows: Vec<String> = env.read(|c| {
        let mut s = c.prepare("SELECT rel_path FROM file_locations WHERE asset_id = ?1").unwrap();
        s.query_map([x[0]], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    });
    assert_eq!(rows.len(), 1, "only the active location in b remains: {rows:?}");
    assert!(rows[0].starts_with("b/"));
    assert_eq!(status_counts(&env, &y, "deleted"), 1);
}

#[test]
fn t_rmv_06_mixed_collection_keeps_the_other_folders_members_and_tag_names_survive() {
    // Arrange
    let env = Env::new();
    for d in ["a", "b"] {
        fs::create_dir_all(env.path(d)).unwrap();
    }
    let (a, b) = (add(&env, "a"), add(&env, "b"));
    let ia = insert_assets(&env, "vol-a", "a", 3, "image");
    let ib = insert_assets(&env, "vol-a", "b", 2, "image");
    seed_linked(&env, &ia);
    seed_linked(&env, &ib);

    // Act: 한 폴더씩 제거 — 컬렉션은 비어도 남고 태그 이름도 남는다
    env.ingest.remove_folder(a).unwrap();
    env.ingest.wait_removals();
    let members_after_a = env.count("SELECT COUNT(*) FROM collection_assets");
    env.ingest.remove_folder(b).unwrap();
    env.ingest.wait_removals();

    // Assert
    assert_eq!(members_after_a, 2);
    assert_eq!(env.count("SELECT COUNT(*) FROM collection_assets"), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM collections WHERE name_key = 'seed-collection'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM tags WHERE name_key = 'seed-tag'"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 0);
}

#[test]
fn t_rmv_07_offline_folder_can_be_removed_without_reading_the_disk() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    let lib = add(&env, "lib");
    scan(&env, lib);
    let ids = ids_under(&env, "lib");
    seed_linked(&env, &ids);
    env.volumes.set_connected("vol-a", false);
    exec(&env, "UPDATE volumes SET availability = 'offline' WHERE volume_key = 'vol-a'");
    let originals = snapshot(&env.vol);

    // Act
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();

    // Assert
    assert_eq!(total_linked(&env, &ids), 0);
    assert_eq!(status_counts(&env, &ids, "deleted"), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 0);
    assert_eq!(snapshot(&env.vol), originals);
}

#[test]
fn t_rmv_08_repeating_the_removal_is_safe_and_adds_no_events() {
    // Arrange
    let env = Env::new();
    fs::create_dir_all(env.path("lib")).unwrap();
    let lib = add(&env, "lib");
    let ids = insert_assets(&env, "vol-a", "lib", 3, "image");
    assert!(env.ingest.begin_folder_removal(lib).unwrap());

    // Act + Assert: 이미 제거 중이면 아무것도 바꾸지 않는다
    assert!(!env.ingest.begin_folder_removal(lib).unwrap());
    assert_eq!(env.ingest.run_folder_removal(lib, &|| false).unwrap(), RemovalRun::Finished);
    let outbox = env.count("SELECT COUNT(*) FROM search_outbox");
    assert_eq!(env.ingest.run_folder_removal(lib, &|| false).unwrap(), RemovalRun::Finished);
    assert!(matches!(env.ingest.remove_folder(lib), Err(IngestError::NotFound)));
    assert_eq!(env.count("SELECT COUNT(*) FROM search_outbox"), outbox);
    assert_eq!(status_counts(&env, &ids, "deleted"), 3);
    // 제거 중이 아닌 폴더에 삭제 루프를 돌리는 것은 거부 (활성 폴더 보호)
    fs::create_dir_all(env.path("live")).unwrap();
    let live = add(&env, "live");
    assert!(matches!(env.ingest.run_folder_removal(live, &|| false), Err(IngestError::InvalidInput(_))));
}

#[test]
fn t_rmv_09_adding_the_same_folder_again_scans_from_scratch_as_new_assets() {
    // Arrange
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("lib/b.jpg", 2);
    let lib = add(&env, "lib");
    scan(&env, lib);
    let old = ids_under(&env, "lib");
    seed_linked(&env, &old);
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();

    // Act
    let again = add(&env, "lib");
    scan(&env, again);

    // Assert: 새 자산 번호, 옛 자산은 deleted 로 남고 태그·메모는 돌아오지 않는다
    let new: Vec<i64> = ids_under(&env, "lib").into_iter().filter(|id| !old.contains(id)).collect();
    assert_eq!(new.len(), 2);
    assert!(new.iter().all(|id| old.iter().all(|o| id > o)));
    assert_eq!(status_counts(&env, &new, "active"), 2);
    assert_eq!(status_counts(&env, &old, "deleted"), 2);
    assert_eq!(total_linked(&env, &new), 0);
    assert_eq!(total_linked(&env, &old), 0);
}

#[test]
fn t_rmv_10_backup_failure_changes_nothing_and_the_folder_stays_usable() {
    // Arrange
    let env = Env::new();
    fs::create_dir_all(env.path("lib")).unwrap();
    let lib = add(&env, "lib");
    let ids = insert_assets(&env, "vol-a", "lib", 3, "image");
    seed_linked(&env, &ids);
    let before = total_linked(&env, &ids);
    let calls = backup_hook(&env, true);
    let jobs_before = env.count("SELECT COUNT(*) FROM jobs");

    // Act
    let err = env.ingest.remove_folder(lib).unwrap_err();
    env.ingest.wait_removals();

    // Assert
    assert!(matches!(err, IngestError::BackupFailed), "{err:?}");
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(folder_ids(&env), vec![lib]);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders WHERE enabled = 1 AND hidden = 0"), 1);
    assert_eq!(total_linked(&env, &ids), before);
    assert_eq!(status_counts(&env, &ids, "active"), 3);
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs"), jobs_before);
    // 백업이 되면 제거가 진행된다
    backup_hook(&env, false);
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();
    assert_eq!(status_counts(&env, &ids, "deleted"), 3);
}

#[test]
fn t_rmv_11_add_relocate_and_rescan_overlapping_a_removing_folder_are_busy() {
    // Arrange: 제거 중인 폴더(삭제 루프는 아직 시작 전)
    let env = Env::new();
    for d in ["p/q", "other"] {
        fs::create_dir_all(env.path(d)).unwrap();
    }
    let (pq, other) = (add(&env, "p/q"), add(&env, "other"));
    env.ingest.begin_folder_removal(pq).unwrap();
    let path = |rel: &str| env.path(rel).to_str().unwrap().to_string();

    // Act + Assert
    assert!(matches!(env.ingest.add_folder(&path("p/q")), Err(IngestError::Busy)), "same path");
    assert!(matches!(env.ingest.add_folder(&path("p")), Err(IngestError::Busy)), "parent");
    fs::create_dir_all(env.path("p/q/deep")).unwrap();
    assert!(matches!(env.ingest.add_folder(&path("p/q/deep")), Err(IngestError::Busy)), "child");
    assert!(matches!(env.ingest.relocate_folder(other, &path("p/q")), Err(IngestError::Busy)));
    assert!(matches!(env.ingest.rescan_folder(pq), Err(IngestError::Busy)));
    assert_eq!(env.ingest.retry_scan_errors(None).unwrap(), 0, "busy folders are skipped, not an error");

    // 삭제가 끝나면 다시 등록할 수 있다
    env.ingest.run_folder_removal(pq, &|| false).unwrap();
    assert!(env.ingest.add_folder(&path("p/q")).is_ok());
}

#[test]
fn t_rmv_12_removal_cancels_pending_jobs_of_the_folder_and_its_assets() {
    // Arrange: add_folder 가 첫 스캔 작업을 예약했고, 자산에는 대기 중인 AI 작업이 있다
    let env = Env::new();
    fs::create_dir_all(env.path("lib")).unwrap();
    let lib = add(&env, "lib");
    let ids = insert_assets(&env, "vol-a", "lib", 2, "image");
    seed_linked(&env, &ids);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM jobs WHERE folder_id = {lib}")), 1);

    // Act
    env.ingest.begin_folder_removal(lib).unwrap();
    let after_begin = env.count(&format!("SELECT COUNT(*) FROM jobs WHERE folder_id = {lib}"));
    env.ingest.run_folder_removal(lib, &|| false).unwrap();

    // Assert
    assert_eq!(after_begin, 0);
    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM jobs WHERE asset_id IN ({})", ids_sql(&ids))), 0);
}
