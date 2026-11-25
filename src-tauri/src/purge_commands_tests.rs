// Categories: Validation (오류 코드·이벤트·직렬화 계약) / Happy (제거 -> 이벤트 -> 목록, 백업 훅 왕복) / External (백업 실패 -> backup_failed) /
// Business rule (관리 폴더 있으면 volume_has_folders, 안전 백업 종류)
// command 는 `blocking`/`run` 에서 `IngestError` 를 `CommandError` 로 바꾸므로 같은 변환으로 계약을 확인한다.
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::scan::IngestError;
use mam_core::storage::DataPaths;
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{BackupKind, Clock, DbPool, SystemClock, migrate, open};
use serde_json::json;

use crate::ai::EventSink;
use crate::error::{CommandError, ErrorCode};
use crate::storage_commands::Storage;
use crate::volume_commands::wire_removal_events;

#[derive(Default)]
struct Capture(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Capture {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().unwrap().push((event.into(), payload));
    }
}

struct FixedClock(AtomicI64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    storage: Arc<Storage>,
    media: std::path::PathBuf,
    ingest: Arc<Ingest>,
}

use std::sync::Arc;

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let paths = DataPaths::new(base.join("MAMData"));
    paths.ensure_layout().unwrap();
    let media = base.join("media");
    std::fs::create_dir_all(&media).unwrap();
    std::fs::write(media.join("a.jpg"), b"a").unwrap();
    let mut conn = open(&paths.db()).unwrap();
    migrate(&mut conn, &paths.backups(), &SystemClock).unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&paths.db(), 2).unwrap());
    let storage = Arc::new(Storage {
        pool: Arc::clone(&pool),
        paths: paths.clone(),
        bootstrap_dir: base.join("boot"),
        _lock: None,
    });
    let ingest = Ingest::new(
        IngestDeps {
            pool,
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![paths.root().to_path_buf()],
            ..IngestConfig::default()
        },
    );
    Fx {
        _dir: dir,
        storage,
        media,
        ingest,
    }
}

#[test]
fn t_ipc_purge_01_new_error_codes_serialize_in_snake_case_and_hide_internals() {
    // Arrange + Act
    let cases = [
        (IngestError::Busy, ErrorCode::Busy, "busy"),
        (IngestError::BackupFailed, ErrorCode::BackupFailed, "backup_failed"),
        (IngestError::VolumeHasFolders, ErrorCode::VolumeHasFolders, "volume_has_folders"),
    ];

    // Assert
    for (err, code, text) in cases {
        let cmd = CommandError::from(err);
        assert_eq!(cmd.code(), code);
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["code"], text);
        assert!(!v["message"].as_str().unwrap().contains('\\'), "no paths in messages");
    }
}

#[test]
fn t_ipc_purge_02_remove_folder_emits_removal_progress_with_the_contract_shape_and_backs_up_first() {
    // Arrange
    let fx = fixture();
    let cap = Arc::new(Capture::default());
    wire_removal_events(&fx.ingest, cap.clone());
    fx.ingest.set_backup_hook(crate::purge_backup::backup_hook(
        Arc::clone(&fx.storage),
        Arc::new(FixedClock(AtomicI64::new(1_800_000_000_000))),
    ));
    let id = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id;

    // Act
    fx.ingest.remove_folder(id).unwrap();
    fx.ingest.wait_removals();

    // Assert: 마지막 이벤트가 { folderId, done, total, finished, error } 모양
    let events = cap.0.lock().unwrap();
    assert!(events.iter().all(|(name, _)| name == "folders://removal-progress"));
    let last = &events.last().unwrap().1;
    assert_eq!(
        *last,
        json!({"folderId": id, "done": 0, "total": 0, "finished": true, "error": null})
    );
    // 사전 백업은 안전 백업 종류(`pre_remove`)로 한 개 만들어졌다
    let backups = mam_db::list_backups(&fx.storage.paths.backups()).unwrap();
    assert_eq!(backups.len(), 1);
    assert_eq!(backups[0].kind, BackupKind::PreRemove);
    assert_eq!(serde_json::to_value(backups[0].kind).unwrap(), "pre_remove");
    assert!(fx.ingest.list_folders().unwrap().is_empty());
}

#[test]
fn t_ipc_purge_03_backup_failure_maps_to_backup_failed_and_the_folder_stays() {
    // Arrange: 백업 폴더 자리에 파일이 있어 폴더를 만들 수 없다
    let fx = fixture();
    let dir = fx.storage.paths.backups();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::write(&dir, b"not a directory").unwrap();
    fx.ingest.set_backup_hook(crate::purge_backup::backup_hook(
        Arc::clone(&fx.storage),
        Arc::new(SystemClock),
    ));
    let id = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id;

    // Act
    let err = CommandError::from(fx.ingest.remove_folder(id).unwrap_err());
    fx.ingest.wait_removals();

    // Assert
    assert_eq!(err.code(), ErrorCode::BackupFailed);
    assert_eq!(fx.ingest.list_folders().unwrap().len(), 1, "nothing changed");
}

#[test]
fn t_ipc_purge_04_known_volumes_and_forget_follow_the_contract() {
    // Arrange: 실제 볼륨에 폴더 하나
    let fx = fixture();
    fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap();
    let list = serde_json::to_value(fx.ingest.list_known_volumes().unwrap()).unwrap();
    let key = list[0]["volumeKey"].as_str().unwrap().to_string();

    // Act
    let refused = CommandError::from(fx.ingest.forget_volume_record(&key).unwrap_err());

    // Assert
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["folderCount"], 1);
    for field in ["label", "kind", "lastMountPoint", "availability", "lastSeenAt", "ignored"] {
        assert!(list[0].get(field).is_some(), "missing {field}");
    }
    assert_eq!(refused.code(), ErrorCode::VolumeHasFolders);
    // 모르는 키는 성공, 빈 키는 invalid_input
    assert!(fx.ingest.forget_volume_record("never-seen").is_ok());
    let empty = CommandError::from(fx.ingest.forget_volume_record("").unwrap_err());
    assert_eq!(empty.code(), ErrorCode::InvalidInput);
}

#[test]
fn t_ipc_purge_05_impact_serializes_with_the_contract_keys() {
    let fx = fixture();
    let id = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id;

    let v = serde_json::to_value(fx.ingest.folder_removal_impact(id).unwrap()).unwrap();

    assert_eq!(
        v,
        json!({"mediaCount": 0, "tagLinks": 0, "notes": 0, "collectionItems": 0, "aiResults": 0})
    );
    let unknown = CommandError::from(fx.ingest.folder_removal_impact(id + 100).unwrap_err());
    assert_eq!(unknown.code(), ErrorCode::NotFound);
}
