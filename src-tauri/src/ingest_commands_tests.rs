// Categories: Validation (계약 모양) / Business rule (오류 코드 매핑·경로 비노출) / External (대화상자 결과 변환)
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::scan::scan_folder;
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DbPool, SystemClock, migrate, open};
use tauri_plugin_dialog::FilePath;

use super::*;

fn keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

struct Fx {
    _dir: tempfile::TempDir,
    media: PathBuf,
    ingest: Arc<Ingest>,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let data = base.join("MAMData");
    let media = base.join("media");
    std::fs::create_dir_all(media.join("sub")).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(media.join("a.jpg"), b"a").unwrap();
    std::fs::write(media.join("sub").join("b.mp4"), b"bb").unwrap();
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
        ingest,
    }
}

#[test]
fn t_ipc_14_responses_have_exactly_the_contract_shapes() {
    // Arrange: 실제 볼륨 API + 실제 스캔
    let fx = fixture();
    let view = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap();
    scan_folder(
        fx.ingest.env(),
        view.folder_id,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap();
    fx.ingest
        .env()
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute(
                "INSERT INTO scan_errors (folder_id, rel_path, error_code, message, last_at)
                 VALUES (1, 'x', 'io', 'm', 5)",
                [],
            )?;
            Ok(())
        })
        .unwrap();

    // Act
    let folders = serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap();
    let nodes = serde_json::to_value(fx.ingest.list_subfolders(1, None).unwrap()).unwrap();
    let summary = serde_json::to_value(fx.ingest.summary().unwrap()).unwrap();
    let (items, total, next_cursor) = fx.ingest.list_scan_errors(None, 10).unwrap();
    let page = serde_json::to_value(Page {
        items,
        total,
        next_cursor,
    })
    .unwrap();

    // Assert: ipc-contract.ts 의 ManagedFolder / FolderNode / JobSummary / Page<ScanError>
    // (`watchHealth` 는 실시간 감시가 변경 신호를 받은 뒤에만 실리는 선택 필드라 키 집합 비교에서 뺀다)
    let mut folder_keys = keys(&folders[0]);
    folder_keys.retain(|k| k != "watchHealth");
    assert_eq!(
        folder_keys,
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
    assert_eq!(folders[0]["assetCount"], 2);
    assert_eq!(folders[0]["availability"], "online");
    assert!(
        ["fixed", "removable", "network"].contains(&folders[0]["volumeKind"].as_str().unwrap())
    );
    assert!(folders[0]["lastFullScanAt"].is_i64());
    assert_eq!(
        folders[0]["scanState"], "scanning",
        "first scan job is still queued"
    );
    assert_eq!(
        keys(&nodes[0]),
        ["assetCount", "hasChildren", "name", "relPath"]
    );
    assert_eq!(nodes[0]["relPath"], "sub");
    assert_eq!(
        keys(&summary),
        ["counts", "paused", "pausedKinds", "scan", "totalAssets"]
    );
    assert_eq!(
        keys(&summary["counts"]),
        ["ai", "hash", "index", "meta", "plugin", "scan", "scene", "thumb"]
    );
    assert_eq!(
        keys(&summary["counts"]["scan"]),
        ["failed", "pending", "running"]
    );
    assert_eq!(summary["counts"]["scan"]["pending"], 1);
    assert!(summary["scan"].is_null());
    assert_eq!(keys(&page), ["items", "nextCursor", "total"]);
    assert_eq!(
        keys(&page["items"][0]),
        [
            "attempts",
            "errorCode",
            "errorId",
            "folderId",
            "lastAt",
            "message",
            "path"
        ]
    );
    assert!(page["nextCursor"].is_null());
}

#[test]
fn t_ipc_15_ingest_errors_map_to_contract_codes_without_leaking_paths() {
    let secret = std::io::Error::other(r"C:\Users\someone\secret");
    let cases: Vec<(IngestError, ErrorCode)> = vec![
        (IngestError::AlreadyRegistered, ErrorCode::AlreadyRegistered),
        (
            IngestError::NestedFolder {
                inside_existing: true,
            },
            ErrorCode::NestedFolder,
        ),
        (
            IngestError::NestedFolder {
                inside_existing: false,
            },
            ErrorCode::NestedFolder,
        ),
        (IngestError::Offline, ErrorCode::Offline),
        (IngestError::NotFound, ErrorCode::NotFound),
        (IngestError::InvalidInput("path"), ErrorCode::InvalidInput),
        (
            IngestError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)),
            ErrorCode::DiskFull,
        ),
        (IngestError::Io(secret), ErrorCode::Io),
        (
            IngestError::Sql(rusqlite::Error::QueryReturnedNoRows),
            ErrorCode::Internal,
        ),
    ];
    for (err, code) in cases {
        let mapped = CommandError::from(err);
        assert_eq!(mapped.code(), code);
        let json = serde_json::to_string(&mapped).unwrap();
        assert!(
            !json.contains("Users") && !json.contains("secret"),
            "{json}"
        );
    }
}

#[test]
fn t_ipc_16_command_arguments_follow_the_contract_names() {
    // pause_jobs / resume_jobs 의 { kind } — 문자열 또는 null
    let k: Option<JobKind> = serde_json::from_value(serde_json::json!("thumb")).unwrap();
    assert_eq!(k, Some(JobKind::Thumb));
    let none: Option<JobKind> = serde_json::from_value(serde_json::Value::Null).unwrap();
    assert_eq!(none, None);
    assert!(serde_json::from_value::<Option<JobKind>>(serde_json::json!("bogus")).is_err());
    for (kind, text) in [
        (JobKind::Scan, "scan"),
        (JobKind::Meta, "meta"),
        (JobKind::Thumb, "thumb"),
        (JobKind::Hash, "hash"),
        (JobKind::Index, "index"),
        (JobKind::Ai, "ai"),
        (JobKind::Scene, "scene"),
        (JobKind::Plugin, "plugin"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), text);
    }
}

#[test]
fn t_ipc_17_folder_commands_surface_validation_errors_to_the_caller() {
    let fx = fixture();
    let first = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap();
    let dup = CommandError::from(
        fx.ingest
            .add_folder(fx.media.to_str().unwrap())
            .unwrap_err(),
    );
    let nested = CommandError::from(
        fx.ingest
            .add_folder(fx.media.join("sub").to_str().unwrap())
            .unwrap_err(),
    );
    let relative = CommandError::from(fx.ingest.add_folder("media").unwrap_err());
    let unknown = CommandError::from(fx.ingest.remove_folder(first.folder_id + 100).unwrap_err());
    let bad_page = CommandError::from(fx.ingest.list_scan_errors(Some("x"), 5).unwrap_err());

    assert_eq!(
        [
            dup.code(),
            nested.code(),
            relative.code(),
            unknown.code(),
            bad_page.code()
        ],
        [
            ErrorCode::AlreadyRegistered,
            ErrorCode::NestedFolder,
            ErrorCode::InvalidInput,
            ErrorCode::NotFound,
            ErrorCode::InvalidInput
        ]
    );
}

#[test]
fn t_ipc_18_pick_directory_result_is_an_absolute_path_or_null() {
    // 취소
    assert_eq!(picked_to_text(None).unwrap(), None);
    // 정상 (확장 접두는 제거해 UI 에 보여 준다)
    let ok = picked_to_text(Some(FilePath::Path(PathBuf::from(r"\\?\D:\Export")))).unwrap();
    assert_eq!(ok.as_deref(), Some(r"D:\Export"));
    // 상대 경로·URL 은 거부
    let rel = picked_to_text(Some(FilePath::Path(PathBuf::from("Export")))).unwrap_err();
    assert_eq!(rel.code(), ErrorCode::InvalidInput);
    let url = tauri::Url::parse("content://tree/primary").unwrap();
    let uri = picked_to_text(Some(FilePath::Url(url))).unwrap_err();
    assert_eq!(uri.code(), ErrorCode::InvalidInput);
}

#[path = "ingest_hide_tests.rs"]
mod hide;

#[path = "ingest_alias_tests.rs"]
mod alias;

#[path = "ingest_mtype_tests.rs"]
mod mtype;

#[test]
fn t_ipc_19_pick_directory_default_path_is_used_only_for_existing_absolute_dirs() {
    // Arrange
    let fx = fixture();

    // Act / Assert: 없음(기존 호출)·존재하지 않음·상대·파일은 무시, 존재하는 절대 디렉터리만 사용
    assert_eq!(existing_dir(None), None);
    assert_eq!(existing_dir(Some(r"Z:\definitely\missing")), None);
    assert_eq!(existing_dir(Some("media")), None);
    assert_eq!(existing_dir(Some(fx.media.join("a.jpg").to_str().unwrap())), None);
    assert_eq!(existing_dir(Some(fx.media.to_str().unwrap())), Some(fx.media.clone()));
}
