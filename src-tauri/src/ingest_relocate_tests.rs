// Categories: Validation (계약 모양) / Business rule (오류 코드 매핑·경로 비노출)
use std::path::PathBuf;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};

use super::*;

struct Fx {
    _dir: tempfile::TempDir,
    media: PathBuf,
    other: PathBuf,
    ingest: Arc<Ingest>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("MAMData");
    let (media, other) = (base.join("media"), base.join("other"));
    std::fs::create_dir_all(media.join("sub")).unwrap();
    std::fs::create_dir_all(other.join("sub")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(media.join("a.jpg"), b"a").unwrap();
    let db = data.join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &data.join("backups"), &SystemClock).unwrap();
    drop(conn);
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::new(DbPool::open(&db, 2).unwrap()),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![data],
            ..IngestConfig::default()
        },
    );
    Fx {
        _dir: dir,
        media,
        other,
        ingest,
    }
}

#[test]
fn t_ipc_21_scan_running_and_same_path_map_to_contract_codes_without_leaking_paths() {
    // Arrange
    let secret = r"C:\Users\someone\secret";
    // Act
    let running = CommandError::from(IngestError::ScanRunning);
    let same = CommandError::from(IngestError::SamePath);
    // Assert: ScanRunning 은 새 코드, SamePath 는 별도 코드 없이 already_registered
    assert_eq!(running.code(), ErrorCode::ScanRunning);
    assert_eq!(same.code(), ErrorCode::AlreadyRegistered);
    for e in [&running, &same] {
        let json = serde_json::to_string(e).unwrap();
        assert!(!json.contains("Users") && !json.contains(secret), "{json}");
    }
    assert_eq!(
        serde_json::to_value(&running).unwrap(),
        serde_json::json!({"code": "scan_running", "message": "a scan of this folder is running"})
    );
}

#[test]
fn t_ipc_22_relocate_outcome_serializes_as_camel_case_folder_and_count() {
    // Arrange: 실제 폴더를 등록하고 다른 경로로 이전한다
    let fx = fixture();
    let first = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap();
    fx.ingest.jobs().pause(Some(JobKind::Scan)).unwrap();

    // Act
    let out = fx
        .ingest
        .relocate_folder(first.folder_id, fx.other.to_str().unwrap())
        .unwrap();
    let value = serde_json::to_value(&out).unwrap();

    // Assert
    let mut top: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    top.sort_unstable();
    assert_eq!(top, ["folder", "relocated"]);
    assert_eq!(
        value["relocated"], 0,
        "nothing was scanned yet, so no location to move"
    );
    let mut keys: Vec<&str> = value["folder"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys.retain(|k| *k != "watchHealth"); // 감시가 신호를 받은 뒤에만 실리는 선택 필드
    assert_eq!(
        keys,
        [
            "alias",
            "assetCount",
            "availability",
            "ejectPrepared",
            "ejectable",
            "folderId",
            "hidden",
            "lastFullScanAt",
            "mediaTypes",
            "nextRescanAt",
            "path",
            "rescanPolicy",
            "scanState",
            "volumeKind",
            "volumeLabel",
            "watchActive",
            "watchFallbackKey",
            "watchMode",
            "watchOptions"
        ]
    );
    assert_eq!(value["folder"]["folderId"], first.folder_id);
    assert!(value["folder"]["path"].as_str().unwrap().ends_with("other"));
}

#[test]
fn t_ipc_23_relocate_validation_errors_reach_the_caller_as_contract_codes() {
    // Arrange
    let fx = fixture();
    let first = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap();
    fx.ingest.add_folder(fx.other.to_str().unwrap()).unwrap();
    let go = |id: i64, p: &std::path::Path| {
        CommandError::from(
            fx.ingest
                .relocate_folder(id, p.to_str().unwrap())
                .unwrap_err(),
        )
    };

    // Act
    let same = go(first.folder_id, &fx.media);
    let other_itself = go(first.folder_id, &fx.other);
    let nested = go(first.folder_id, &fx.media.join("sub"));
    let into_other = go(first.folder_id, &fx.other.join("sub"));
    let unknown = go(first.folder_id + 100, &fx.media);

    // Assert
    assert_eq!(
        [
            same.code(),
            other_itself.code(),
            nested.code(),
            into_other.code(),
            unknown.code()
        ],
        [
            ErrorCode::AlreadyRegistered,
            ErrorCode::AlreadyRegistered,
            ErrorCode::NestedFolder,
            ErrorCode::NestedFolder,
            ErrorCode::NotFound
        ]
    );
}
