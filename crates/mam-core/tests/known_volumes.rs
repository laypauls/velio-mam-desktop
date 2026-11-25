//! 기억하는 디스크 목록·기록 지우기 (T-known-01 ~ 09).
//!
//! 분류: Happy (01 병합 목록, 03 지우기) / Validation (06 빈 키·모르는 키) / Business rule (02 폴더 수, 04 폴더 있으면 거부,
//! 05 묻지 않음만 있는 항목, 07 옛 보관 자산, 08 다시 묻기) / Boundary (09 직렬화). 실제 SQLite + 가짜 볼륨 + 수동 시계.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};

use common::env::Env;
use common::purge::{exec, insert_assets, seed_linked, status_counts, total_linked, volume_id};
use mam_core::VolumeKind;
use mam_core::ingest::KnownVolume;
use mam_core::scan::IngestError;
use mam_db::Priority;
use mam_db::ignored_volumes::{IgnoredVolume, add_ignored_volume, load_ignored_volumes};

fn add(env: &Env, rel: &str) -> i64 {
    fs::create_dir_all(env.path(rel)).unwrap();
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn ignore(env: &Env, key: &str, label: &str) {
    let entry = IgnoredVolume {
        volume_key: key.into(),
        label: Some(label.into()),
        last_mount_point: Some(r"H:\".into()),
        ignored_at: 7,
    };
    env.pool
        .write(Priority::High, move |c| Ok(add_ignored_volume(c, &entry)))
        .unwrap()
        .unwrap();
}

fn known(env: &Env) -> Vec<KnownVolume> {
    env.ingest.list_known_volumes().unwrap()
}

fn volume_rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM volumes")
}

/// 폴더를 등록했다가 삭제까지 마쳐 "관리 폴더 없는 디스크" 로 만든다
fn emptied_volume(env: &Env) {
    let lib = add(env, "lib");
    env.ingest.remove_folder(lib).unwrap();
    env.ingest.wait_removals();
}

#[test]
fn t_known_01_list_merges_volume_rows_and_ignored_entries_one_row_per_key() {
    // Arrange: vol-a (폴더 있음, 연결됨), vol-a 도 묻지 않음에 있음, 행 없는 묻지 않음 항목 하나
    let env = Env::new();
    add(&env, "lib");
    ignore(&env, "vol-a", "ignored copy");
    ignore(&env, "usb-old", "OLD USB");

    // Act
    let list = known(&env);

    // Assert
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].volume_key, "vol-a");
    assert_eq!(list[0].kind, Some(mam_core::VolumeKind::Fixed));
    assert_eq!(list[0].folder_count, 1);
    assert!(list[0].ignored);
    assert_eq!(list[0].label.as_deref(), Some("vol-a"), "volumes row wins over the ignored entry");
    assert!(list[0].last_seen_at.is_some());
    let old = &list[1];
    assert_eq!(old.volume_key, "usb-old");
    assert_eq!((old.kind, old.availability, old.last_seen_at), (None, None, None));
    assert_eq!((old.label.as_deref(), old.last_mount_point.as_deref(), old.folder_count, old.ignored), (Some("OLD USB"), Some(r"H:\"), 0, true));
}

#[test]
fn t_known_02_folder_count_excludes_folders_being_removed_but_forget_still_refuses() {
    // Arrange
    let env = Env::new();
    let (a, _b) = (add(&env, "a"), add(&env, "b"));
    assert_eq!(known(&env)[0].folder_count, 2);

    // Act
    env.ingest.begin_folder_removal(a).unwrap();

    // Assert
    assert_eq!(known(&env)[0].folder_count, 1);
    env.ingest.run_folder_removal(a, &|| false).unwrap();
    assert!(matches!(env.ingest.forget_volume_record("vol-a"), Err(IngestError::VolumeHasFolders)));
}

#[test]
fn t_known_03_forgetting_a_disk_without_folders_removes_row_ignored_entry_and_old_locations_only() {
    // Arrange: 폴더 제거 뒤 남은 디스크 + 삭제된 자산의 옛 위치 행 + 묻지 않음 항목
    let env = Env::new();
    emptied_volume(&env);
    let vid = volume_id(&env, "vol-a");
    let gone = insert_assets(&env, "vol-a", "x", 1, "image");
    seed_linked(&env, &gone);
    exec(&env, &format!(
        "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {0};
         UPDATE file_locations SET detached_at = 5 WHERE volume_id = {vid};", gone[0]));
    ignore(&env, "vol-a", "x");
    let linked_before = total_linked(&env, &gone);
    let calls = Arc::new(Mutex::new(0));
    let c = Arc::clone(&calls);
    env.ingest.set_backup_hook(Arc::new(move || { *c.lock().unwrap() += 1; Ok(()) }));

    // Act
    env.ingest.forget_volume_record("vol-a").unwrap();

    // Assert: 위치 행·volumes 행·묻지 않음 항목이 사라지고, 삭제된 자산의 연결 데이터는 건드리지 않았다 (백업도 필요 없었다)
    assert_eq!(volume_rows(&env), 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM file_locations"), 0);
    assert!(env.read(|c| load_ignored_volumes(c).unwrap()).is_empty());
    assert_eq!(total_linked(&env, &gone), linked_before);
    assert_eq!(*calls.lock().unwrap(), 0);
    assert!(known(&env).is_empty());
    assert_eq!(status_counts(&env, &gone, "deleted"), 1, "the asset row itself stays");
}

#[test]
fn t_known_04_forget_with_folders_is_rejected_and_changes_nothing() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    ignore(&env, "vol-a", "x");

    // Act
    let err = env.ingest.forget_volume_record("vol-a").unwrap_err();

    // Assert
    assert!(matches!(err, IngestError::VolumeHasFolders), "{err:?}");
    assert_eq!(volume_rows(&env), 1);
    assert_eq!(env.read(|c| load_ignored_volumes(c).unwrap()).len(), 1);
    assert_eq!(env.count("SELECT COUNT(*) FROM managed_folders"), 1);
}

#[test]
fn t_known_05_ignored_only_entry_is_removed_alone() {
    // Arrange
    let env = Env::new();
    add(&env, "lib");
    ignore(&env, "usb-old", "OLD");

    // Act
    env.ingest.forget_volume_record("usb-old").unwrap();

    // Assert
    assert!(env.read(|c| load_ignored_volumes(c).unwrap()).is_empty());
    assert_eq!(volume_rows(&env), 1, "other volume rows are untouched");
    assert_eq!(known(&env).len(), 1);
}

#[test]
fn t_known_06_unknown_key_is_a_noop_and_invalid_keys_are_rejected() {
    let env = Env::new();
    add(&env, "lib");

    assert!(env.ingest.forget_volume_record("never-seen").is_ok());
    assert!(matches!(env.ingest.forget_volume_record(""), Err(IngestError::InvalidInput(_))));
    assert!(matches!(env.ingest.forget_volume_record("a\0b"), Err(IngestError::InvalidInput(_))));
    assert_eq!(volume_rows(&env), 1);
}

#[test]
fn t_known_07_leftover_legacy_assets_on_the_disk_are_purged_with_a_backup_before_the_record_goes() {
    // Arrange: 폴더는 없지만 활성 자산(detach 된 위치)이 남은 디스크 — 아직 1회 정리가 안 돈 상태
    let env = Env::new();
    emptied_volume(&env);
    let legacy = insert_assets(&env, "vol-a", "gone", 2, "image");
    seed_linked(&env, &legacy);
    exec(&env, "UPDATE file_locations SET detached_at = 5");
    let calls = Arc::new(Mutex::new(0));
    let c = Arc::clone(&calls);
    env.ingest.set_backup_hook(Arc::new(move || { *c.lock().unwrap() += 1; Ok(()) }));

    // Act
    env.ingest.forget_volume_record("vol-a").unwrap();

    // Assert
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(total_linked(&env, &legacy), 0);
    assert_eq!(status_counts(&env, &legacy, "deleted"), 2);
    assert_eq!(volume_rows(&env), 0);
    // 백업이 실패하면 아무것도 지우지 않는다
    let env2 = Env::new();
    emptied_volume(&env2);
    let l2 = insert_assets(&env2, "vol-a", "gone", 1, "image");
    seed_linked(&env2, &l2);
    exec(&env2, "UPDATE file_locations SET detached_at = 5");
    env2.ingest.set_backup_hook(Arc::new(|| Err("full".into())));
    assert!(matches!(env2.ingest.forget_volume_record("vol-a"), Err(IngestError::BackupFailed)));
    assert_eq!(volume_rows(&env2), 1);
    assert_eq!(status_counts(&env2, &l2, "active"), 1);
}

#[test]
fn t_known_08_a_forgotten_disk_that_is_connected_is_asked_again_like_a_new_disk() {
    // Arrange: 첫 점검(baseline) 뒤 관리 폴더를 등록·제거 -> 지금은 폴더 없는 연결된 디스크
    let env = Env::new();
    env.ingest.supervise_once().unwrap();
    emptied_volume(&env);
    env.ingest.supervise_once().unwrap();
    assert!(env.ingest.pending_volumes().is_empty(), "removing the last folder does not prompt by itself");
    let events: Arc<Mutex<Vec<usize>>> = Arc::default();
    let sink = Arc::clone(&events);
    env.ingest.set_pending_listener(Arc::new(move |v| sink.lock().unwrap().push(v.len())));

    // Act
    env.ingest.forget_volume_record("vol-a").unwrap();

    // Assert
    let pending = env.ingest.pending_volumes();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].volume_key, "vol-a");
    assert_eq!(pending[0].kind, VolumeKind::Fixed);
    assert_eq!(events.lock().unwrap().as_slice(), [1]);
    assert_eq!(volume_rows(&env), 0, "a pending disk never enters the volumes table");
}

#[test]
fn t_known_09_known_volume_serializes_with_the_contract_names() {
    let env = Env::new();
    add(&env, "lib");
    ignore(&env, "usb-old", "OLD");
    let json = serde_json::to_value(known(&env)).unwrap();

    assert_eq!(json[0]["volumeKey"], "vol-a");
    assert_eq!(json[0]["kind"], "fixed");
    assert_eq!(json[0]["availability"], "online");
    assert_eq!(json[0]["folderCount"], 1);
    assert_eq!(json[0]["ignored"], false);
    assert!(json[0]["lastSeenAt"].is_i64());
    assert_eq!(
        json[1],
        serde_json::json!({"volumeKey": "usb-old", "label": "OLD", "kind": null, "lastMountPoint": "H:\\",
            "availability": null, "lastSeenAt": null, "folderCount": 0, "ignored": true})
    );
}
