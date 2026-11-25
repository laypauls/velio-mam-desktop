//! 플러그인 IPC 계약 시험 (T-ipc-plugin-*): 오류 매핑·경로 비노출·DTO 직렬화(공유 fixture)·권한 목록.
//! 분류: Validation (계약) / Business rule (경로·플러그인 텍스트 비노출) / External (IO 실패)
//! fixture `plugin_ipc_fixture.json` 은 프론트 `ipc-plugin.test.ts` 도 읽는다 — 한쪽만 바꾸면 이 시험이 깨진다.

use mam_plugin_runtime::{
    AssetKind, ChangeReason, CommandInfo, EngineError, ExportProviderInfo, ExportResult,
    InstallChange, InstallError, InstalledSummary, InvokeError, LocaleError, ManifestError,
    MenuItemInfo, PLUGIN_CHANGED_EVENT, Permission, PluginChanged, PluginCommandResult,
    PluginContributions, PluginInfo, PluginPreview, PluginProviderInfo, PluginStatus,
    PluginTaskCount, ProviderKind, RunAllResult, RunError, ScopeTooLarge,
};
use serde_json::{Value, json};

use crate::error::{CommandError, ErrorCode};

fn fixture() -> Value {
    // 도구가 새로 만든 파일은 CRLF 일 수 있다 — 습관대로 정규화
    serde_json::from_str(&include_str!("plugin_ipc_fixture.json").replace("\r\n", "\n"))
        .expect("fixture parses")
}

fn ser<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap()
}

fn p(perms: &[Permission]) -> Vec<Permission> {
    perms.to_vec()
}

fn info(id: &str) -> PluginInfo {
    PluginInfo {
        id: id.into(),
        version: "1.2.0".into(),
        display_name: "EXIF summary".into(),
        description: Some("Summarizes EXIF data".into()),
        status: PluginStatus::Ok,
        enabled: true,
        unsigned: true,
        permissions: p(&[
            Permission::AssetReadMetadata,
            Permission::KvWrite,
            Permission::TagWrite,
            Permission::ExportWrite,
        ]),
        granted: p(&[Permission::AssetReadMetadata, Permission::KvWrite]),
        commands: vec![CommandInfo {
            id: "summarize".into(),
            title: "Summarize".into(),
        }],
        providers: vec![
            PluginProviderInfo {
                id: "exif".into(),
                title: "EXIF".into(),
                kind: ProviderKind::Metadata,
                asset_kinds: vec![AssetKind::Image],
                file_extension: None,
            },
            PluginProviderInfo {
                id: "csv".into(),
                title: "CSV export".into(),
                kind: ProviderKind::Export,
                asset_kinds: vec![],
                file_extension: Some("csv".into()),
            },
        ],
        installed_at: 1_700_000_000_000,
        worker_available: true,
    }
}

#[test]
fn t_ipc_plugin_01_every_dto_serializes_to_the_shared_fixture() {
    // Arrange
    let fx = fixture();
    let broken = PluginInfo {
        id: "com.example.gone".into(),
        version: "0.0.0".into(),
        display_name: "com.example.gone".into(),
        description: None,
        status: PluginStatus::Broken,
        enabled: false,
        permissions: vec![],
        granted: vec![],
        commands: vec![],
        providers: vec![],
        worker_available: false,
        ..info("com.example.gone")
    };
    let preview = PluginPreview {
        id: "com.example.exif-summary".into(),
        version: "1.3.0".into(),
        display_name: "EXIF summary".into(),
        description: None,
        permissions: p(&[
            Permission::AssetReadMetadata,
            Permission::KvWrite,
            Permission::TagWrite,
        ]),
        commands: vec![CommandInfo {
            id: "summarize".into(),
            title: "Summarize".into(),
        }],
        providers: vec![PluginProviderInfo {
            id: "exif".into(),
            title: "EXIF".into(),
            kind: ProviderKind::Analysis,
            asset_kinds: vec![AssetKind::Image, AssetKind::Video, AssetKind::Audio],
            file_extension: None,
        }],
        unsigned: true,
        installed: Some(InstalledSummary {
            version: "1.2.0".into(),
            granted: p(&[Permission::AssetReadMetadata, Permission::KvWrite]),
        }),
        change: InstallChange::Upgrade,
        added_permissions: p(&[Permission::TagWrite]),
        has_kept_data: false,
    };
    let contributions = PluginContributions {
        menu_items: vec![
            MenuItemInfo {
                plugin_id: "com.example.exif-summary".into(),
                command_id: "summarize".into(),
                title: "Summarize".into(),
                when: Some("asset.kind == 'image'".into()),
            },
            MenuItemInfo {
                plugin_id: "com.example.exif-summary".into(),
                command_id: "other".into(),
                title: "other".into(),
                when: None,
            },
        ],
        export_providers: vec![ExportProviderInfo {
            plugin_id: "com.example.exif-summary".into(),
            provider_id: "csv".into(),
            title: "CSV export".into(),
            file_extension: "csv".into(),
            asset_kinds: vec![AssetKind::Image],
        }],
    };
    let changed = PluginChanged {
        plugin_id: "com.example.exif-summary".into(),
        reason: ChangeReason::AutoDisabled,
    };

    // Act / Assert
    assert_eq!(ser(&info("com.example.exif-summary")), fx["pluginInfo"]);
    assert_eq!(ser(&broken), fx["pluginInfoBroken"]);
    assert_eq!(ser(&preview), fx["pluginPreview"]);
    assert_eq!(ser(&contributions), fx["pluginContributions"]);
    assert_eq!(
        ser(&PluginCommandResult {
            message: Some("done".into())
        }),
        fx["commandResult"]
    );
    assert_eq!(
        ser(&PluginCommandResult { message: None }),
        fx["commandResultNull"]
    );
    assert_eq!(ser(&RunAllResult { started: true }), fx["runAllResult"]);
    assert_eq!(
        ser(&ExportResult {
            bytes_written: 1234
        }),
        fx["exportResult"]
    );
    assert_eq!(
        ser(&vec![PluginTaskCount {
            plugin_id: "com.example.exif-summary".into(),
            pending: 12,
            failed: 1
        }]),
        fx["taskCounts"]
    );
    assert_eq!(ser(&changed), fx["changedEvent"]);
    assert_eq!(fx["eventName"], PLUGIN_CHANGED_EVENT);
}

#[test]
fn t_ipc_plugin_02_permission_list_change_reasons_and_error_codes_match_the_fixture() {
    // Arrange
    let fx = fixture();
    let reasons = [
        ChangeReason::Installed,
        ChangeReason::Upgraded,
        ChangeReason::Uninstalled,
        ChangeReason::Enabled,
        ChangeReason::Disabled,
        ChangeReason::AutoDisabled,
        ChangeReason::PermissionsChanged,
    ];
    let codes = [
        ErrorCode::PluginInvalid,
        ErrorCode::PluginIncompatible,
        ErrorCode::PluginDisabled,
        ErrorCode::PluginPermissionDenied,
        ErrorCode::PluginTimeout,
        ErrorCode::PluginError,
    ];

    // Act / Assert
    let perms: Vec<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
    assert_eq!(
        json!(perms),
        fx["permissions"],
        "Permission::ALL must equal the TS PluginPermission list"
    );
    assert_eq!(ser(&reasons.to_vec()), fx["changeReasons"]);
    assert_eq!(ser(&codes.to_vec()), fx["errorCodes"]);
}

fn code_of(e: impl Into<CommandError>) -> (ErrorCode, String) {
    let e: CommandError = e.into();
    let v = serde_json::to_value(&e).unwrap();
    (e.code(), v["message"].as_str().unwrap().to_owned())
}

#[test]
fn t_ipc_plugin_03_run_errors_map_to_contract_codes_without_plugin_text() {
    // Arrange: 플러그인이 만든 텍스트(경로 포함)가 실린 오류
    let secret = r"C:\Users\someone\AppData\secret.js: boom";
    let cases: Vec<(RunError, ErrorCode)> = vec![
        (RunError::NotInstalled, ErrorCode::NotFound),
        (RunError::UnknownTarget, ErrorCode::NotFound),
        (RunError::Disabled, ErrorCode::PluginDisabled),
        (
            RunError::Invoke(InvokeError::Disabled),
            ErrorCode::PluginDisabled,
        ),
        (
            RunError::PermissionDenied(Permission::ExportWrite),
            ErrorCode::PluginPermissionDenied,
        ),
        (RunError::Scope(ScopeTooLarge), ErrorCode::InvalidInput),
        (
            RunError::Invoke(InvokeError::Timeout),
            ErrorCode::PluginTimeout,
        ),
        (
            RunError::Invoke(InvokeError::Script(secret.into())),
            ErrorCode::PluginError,
        ),
        (
            RunError::Invoke(InvokeError::Plugin {
                code: "busy".into(),
                message: secret.into(),
            }),
            ErrorCode::PluginError,
        ),
        (
            RunError::Invoke(InvokeError::Start(secret.into())),
            ErrorCode::PluginError,
        ),
        (
            RunError::Invoke(InvokeError::Crashed),
            ErrorCode::PluginError,
        ),
    ];

    // Act / Assert
    for (err, want) in cases {
        let label = format!("{err:?}");
        let (code, message) = code_of(err);
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
fn t_ipc_plugin_04_install_errors_map_to_contract_codes_without_host_paths() {
    // Arrange
    let long_stem = "x".repeat(300);
    let cases: Vec<(InstallError, ErrorCode)> = vec![
        (InstallError::NotAbsolute, ErrorCode::InvalidInput),
        (InstallError::NotADirectory, ErrorCode::InvalidInput),
        (InstallError::OverlapsDataDir, ErrorCode::InvalidInput),
        (InstallError::PreviewMismatch, ErrorCode::InvalidInput),
        (InstallError::UnknownPermission, ErrorCode::InvalidInput),
        (InstallError::ManifestMissing, ErrorCode::PluginInvalid),
        (
            InstallError::Manifest(ManifestError::MissingField { path: "id".into() }),
            ErrorCode::PluginInvalid,
        ),
        (
            InstallError::Manifest(ManifestError::UnsupportedApiVersion { found: "9".into() }),
            ErrorCode::PluginIncompatible,
        ),
        (
            InstallError::Engine(EngineError::Incompatible {
                required: ">=9".into(),
                app: "0.1.0".into(),
            }),
            ErrorCode::PluginIncompatible,
        ),
        (
            InstallError::Locale {
                file_stem: long_stem,
                source: LocaleError::TooLarge,
            },
            ErrorCode::PluginInvalid,
        ),
        (InstallError::MainMissing, ErrorCode::PluginInvalid),
        (InstallError::MainEscapes, ErrorCode::PluginInvalid),
        (InstallError::MainTooLarge, ErrorCode::PluginInvalid),
        (InstallError::MainNotUtf8, ErrorCode::PluginInvalid),
        (InstallError::TooManyLocaleFiles, ErrorCode::PluginInvalid),
        (InstallError::NotInstalled, ErrorCode::NotFound),
        (InstallError::Busy, ErrorCode::Busy),
        (
            InstallError::Io(std::io::Error::other(r"C:\Users\someone\src\plugin")),
            ErrorCode::Io,
        ),
        (
            InstallError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)),
            ErrorCode::DiskFull,
        ),
        (
            InstallError::Io(std::io::Error::from_raw_os_error(112)),
            ErrorCode::DiskFull,
        ),
    ];

    // Act / Assert
    for (err, want) in cases {
        let label: String = format!("{err:?}").chars().take(60).collect();
        let (code, message) = code_of(err);
        assert_eq!(code, want, "{label}");
        assert!(
            !message.contains("Users") && !message.contains(":\\"),
            "{label}: {message}"
        );
        assert!(message.chars().count() < 400, "{label}: message too long");
    }
}

#[test]
fn t_ipc_plugin_05_db_errors_use_the_existing_pool_mapping() {
    // Arrange / Act
    let (code, _) = code_of(InstallError::Db(mam_db::PoolError::WriterGone));

    // Assert
    assert_eq!(code, ErrorCode::Internal);
}
