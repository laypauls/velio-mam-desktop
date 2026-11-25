//! P5 command 오류 매핑·인자 검증 시험 (T-ipc-pjob-*) — 워커·DB 없이. 오류 코드 표는 명세 §C3.
//! 분류: Validation (인자) / Business rule (경로·플러그인 텍스트 비노출) / External (IO 실패)

use mam_plugin_runtime::jobs::{ExportError, PluginJobsError, RunAllError};
use mam_plugin_runtime::{InvokeError, Permission, RunAllResult, RunError, ScopeTooLarge};

use super::{export_error, id_arg, retry_error, run_all_error};
use crate::error::{CommandError, ErrorCode};

fn parts(e: CommandError) -> (ErrorCode, String) {
    let v = serde_json::to_value(&e).unwrap();
    (e.code(), v["message"].as_str().unwrap().to_owned())
}

#[test]
fn t_ipc_pjob_01_id_arg_rejects_empty_and_nul() {
    // Arrange / Act / Assert
    assert!(id_arg("exif-plugin").is_ok());
    for bad in ["", "a\0b"] {
        let e = id_arg(bad).unwrap_err();
        assert_eq!(e.code(), ErrorCode::InvalidInput, "{bad:?}");
    }
}

#[test]
fn t_ipc_pjob_02_export_errors_map_to_contract_codes() {
    // Arrange: §C3 export 오류 표 — 플러그인 텍스트·호스트 경로가 응답에 실리면 안 된다
    let secret = r"C:\Users\someone\secret.js: boom";
    let cases: Vec<(ExportError, ErrorCode)> = vec![
        (
            ExportError::InvalidInput("destPath is inside a managed folder"),
            ErrorCode::InvalidInput,
        ),
        (
            ExportError::Run(RunError::UnknownTarget),
            ErrorCode::NotFound,
        ),
        (
            ExportError::Run(RunError::NotInstalled),
            ErrorCode::NotFound,
        ),
        (
            ExportError::Run(RunError::Disabled),
            ErrorCode::PluginDisabled,
        ),
        (
            ExportError::Run(RunError::PermissionDenied(Permission::ExportWrite)),
            ErrorCode::PluginPermissionDenied,
        ),
        (
            ExportError::Run(RunError::Invoke(InvokeError::Timeout)),
            ErrorCode::PluginTimeout,
        ),
        (
            ExportError::Run(RunError::Invoke(InvokeError::Script(secret.into()))),
            ErrorCode::PluginError,
        ),
        (
            ExportError::Run(RunError::Scope(ScopeTooLarge)),
            ErrorCode::InvalidInput,
        ),
        (
            ExportError::BadOutput("output too large"),
            ErrorCode::PluginError,
        ),
        (
            ExportError::Io(std::io::Error::other(secret)),
            ErrorCode::Io,
        ),
        (
            ExportError::Io(std::io::Error::from_raw_os_error(112)),
            ErrorCode::DiskFull,
        ),
        (ExportError::Folders(secret.into()), ErrorCode::Internal),
    ];

    // Act / Assert
    for (err, want) in cases {
        let label = format!("{err:?}");
        let (code, message) = parts(export_error(err));
        assert_eq!(code, want, "{label}");
        assert!(
            !message.contains("Users") && !message.contains("secret") && !message.contains("boom"),
            "{label}: {message}"
        );
        if code == ErrorCode::PluginError {
            assert!(
                message.starts_with("plugin.error."),
                "static i18n key expected: {message}"
            );
        }
    }
}

#[test]
fn t_ipc_pjob_03_run_all_and_retry_errors_map_to_contract_codes() {
    // Arrange / Act / Assert
    assert_eq!(
        parts(run_all_error(RunAllError::Run(RunError::UnknownTarget))).0,
        ErrorCode::NotFound
    );
    assert_eq!(
        parts(run_all_error(RunAllError::Run(RunError::NotInstalled))).0,
        ErrorCode::NotFound
    );
    assert_eq!(
        parts(run_all_error(RunAllError::Run(RunError::Disabled))).0,
        ErrorCode::PluginDisabled
    );
    assert_eq!(
        parts(run_all_error(RunAllError::ExportProvider)).0,
        ErrorCode::InvalidInput
    );
    let (code, msg) = parts(run_all_error(RunAllError::Spawn(std::io::Error::other(
        r"C:\secret",
    ))));
    assert_eq!(code, ErrorCode::Internal);
    assert!(!msg.contains("secret"));
    assert_eq!(
        parts(retry_error(PluginJobsError::NotInstalled)).0,
        ErrorCode::NotFound
    );
}

#[test]
fn t_ipc_pjob_04_results_serialize_in_camel_case() {
    // Arrange / Act / Assert: TS 계약 `RunAllResult { started }`
    assert_eq!(
        serde_json::to_value(RunAllResult { started: true }).unwrap(),
        serde_json::json!({ "started": true })
    );
}

struct FixedClock;

impl mam_db::Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        1_800_000_000_000
    }
}

fn storage_for_protection() -> (
    tempfile::TempDir,
    std::sync::Arc<crate::storage_commands::Storage>,
) {
    let dir = tempfile::TempDir::new().unwrap();
    let paths = mam_core::storage::DataPaths::new(dir.path().join("MAMData"));
    paths.ensure_layout().unwrap();
    let mut conn = mam_db::open(&paths.db()).unwrap();
    mam_db::migrate(&mut conn, &paths.backups(), &FixedClock).unwrap();
    drop(conn);
    let pool = std::sync::Arc::new(mam_db::DbPool::open(&paths.db(), 2).unwrap());
    let storage = crate::storage_commands::Storage {
        paths,
        pool,
        bootstrap_dir: dir.path().join("boot"),
        _lock: None,
    };
    (dir, std::sync::Arc::new(storage))
}

#[test]
fn t_ipc_pjob_05_protected_source_follows_the_current_backup_dir() {
    // Arrange: 부팅 때 만든 source 로 시작해, 그 뒤 설정에서 백업 폴더를 바꾼다
    let (dir, storage) = storage_for_protection();
    let root = storage.paths.root().to_path_buf();
    let source = super::plugin_protected_source(std::sync::Arc::clone(&storage), root.clone());
    let default_backups = storage.paths.backups();
    let before = source();
    let custom = dir.path().join("MyBackups");
    let patch = serde_json::json!({ "backup": { "dir": custom.to_str().unwrap() } });

    // Act
    storage
        .update_settings(
            patch.as_object().unwrap().clone(),
            &mam_core::storage::SystemDriveProbe,
            &[],
        )
        .unwrap();
    let after = source();

    // Assert: 새 백업 폴더가 보호 목록에 들어오고, MAMData 루트는 그대로
    assert!(before.contains(&root) && before.contains(&default_backups));
    assert!(after.contains(&root));
    assert!(after.contains(&custom), "{after:?}");
}
