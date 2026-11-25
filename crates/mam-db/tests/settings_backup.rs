//! 설정·백업 통합 테스트 (T-store-08 ~ T-store-11, P2-1).
//!
//! 분류: Happy (08 기본값·갱신·재시작 유지, 10 백업·보관) / Validation (09 잘못된 값·모르는 키) /
//! External (11 쓰기 실패) / Boundary (목록 필터).

#![recursion_limit = "256"] // 계약 JSON 리터럴이 커서 json! 기본 재귀 한도(128)를 넘는다

mod common;

use std::path::{Path, PathBuf};

use common::{FixedClock, T0, insert_asset, migrated_db};
use mam_db::settings;
use mam_db::{
    BackupKind, DbError, SAFETY_BACKUPS_KEPT, Settings, SettingsError, create_backup,
    create_backup_with, list_backups, open,
};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

fn patch(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn backup(db: &Path, dir: &Path, kind: BackupKind, keep: usize, ms: i64) -> mam_db::BackupInfo {
    create_backup(db, dir, kind, keep, &[], &FixedClock(ms)).unwrap()
}

#[test]
fn t_store_08_defaults_update_and_persist_across_reopen() {
    // Arrange
    let mut db = migrated_db();
    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(loaded, Settings::default());
    assert!(warnings.is_empty());
    // 계약 기본값 (PL1 5분, PL9 24시간·7개, P2-2 재스캔 6시간)
    assert_eq!(loaded.auth_exempt_minutes, 5);
    assert_eq!(loaded.rescan_interval_hours, 6);
    assert!(loaded.backup.enabled);
    assert_eq!((loaded.backup.interval_hours, loaded.backup.keep), (24, 7));
    assert_eq!(loaded.backup.dir, None);
    assert!(loaded.close_to_tray && !loaded.first_run_completed);

    // Act: 일부만 갱신 (중첩 backup 도 부분)
    let updated = settings::update(
        &mut db.conn,
        &patch(json!({"authExemptMinutes": 15, "backup": {"dir": "D:\\Backups"}})),
    )
    .unwrap();
    drop(db.conn);
    let reopened = open(&db.db_path).unwrap();
    let (after, _) = settings::load(&reopened).unwrap();

    // Assert
    assert_eq!(updated.auth_exempt_minutes, 15);
    assert_eq!(after, updated);
    assert_eq!(after.backup.dir.as_deref(), Some("D:\\Backups"));
    assert_eq!(after.backup.keep, 7, "untouched keys keep their defaults");
}

#[test]
fn t_store_08b_serializes_to_the_frontend_contract_shape() {
    let value = serde_json::to_value(Settings::default()).unwrap();
    assert_eq!(
        value,
        json!({
            "rescanIntervalHours": 6,
            "maxConcurrentScans": 3,
            "authExemptMinutes": 5,
            "backup": {"enabled": true, "intervalHours": 24, "keep": 7, "dir": null},
            "closeToTray": true,
            "firstRunCompleted": false,
            "titleEditEnabled": false,
            "originalActionsEnabled": false,
            "sortFieldsEnabled": {"capturedAt": true, "modifiedAt": true, "name": true, "size": true, "duration": true, "rating": true},
            "doubleClickAction": "preview",
            "similarMaxResults": 24,
            "similarMinSimilarity": 0.75,
            "galleryPaging": "infinite",
            "galleryPageSize": 100,
            "indexRail": "auto",
            "theme": {"mode": "system", "systemDark": "dim"},
            "capturedAtFallback": "modified",
            "sidebarRatingMenu": true,
            "videoPlayerVlc": true,
            "viewerPlacement": "left",
            "language": "en",
            "hideShortcut": "Escape",
            "closeViewerShortcut": "Q",
            "slideshowImageSeconds": 5,
            "mediaOpenAutoplay": false,
            "playRangeKeys": "brackets",
            "playRangesOnly": false,
            "collectionPlayRangesOnly": false,
            "subtitleVisible": true,
            "subtitleDefaultLanguage": "ko",
            "sceneCutsVisible": true,
            "sceneDetectAuto": false,
            "sceneDetectSensitivity": "medium",
            "playerPlayingStepSec": 5,
            "resumeEnabled": true,
            "resumeMinDurationSec": 180,
            "layout": {"detailOpen": true, "sidebarOpen": true, "viewerWidth": 520, "detailTab": "info", "filmstripHeight": 104},
            "playerVolume": {"level": 100, "muted": false},
            "galleryCardDisplay": {
                "info": {"typeIcon": true, "name": true, "ext": true, "resolution": true, "size": true, "date": true},
                "dateBy": "captured",
                "check": "hover",
                "cardSize": "m",
                "favorite": true,
                "rating": true,
                "duration": true,
                "quickActions": true
            }
        })
    );
}

#[test]
fn t_store_09_invalid_stored_value_falls_back_to_default_and_unknown_key_is_ignored() {
    // Arrange
    let db = migrated_db();
    for (k, v) in [
        ("auth_exempt_minutes", "\"soon\""),
        ("backup_keep", "0"),
        ("backup_interval_hours", "not json"),
        ("close_to_tray", "1"),
        ("backup_dir", "\"relative\\\\dir\""),
        ("from_a_future_version", "42"),
    ] {
        db.conn
            .execute("INSERT INTO settings (key, value) VALUES (?1, ?2)", [k, v])
            .unwrap();
    }

    // Act
    let (loaded, warnings) = settings::load(&db.conn).unwrap();

    // Assert
    assert_eq!(loaded, Settings::default());
    assert_eq!(warnings.len(), 5, "{warnings:?}");
    assert!(
        warnings.iter().all(|w| !w.contains("soon")),
        "values are not logged"
    );
}

#[test]
fn t_store_09b_patch_rejects_unknown_keys_and_bad_values_atomically() {
    // Arrange
    let mut db = migrated_db();
    let cases = [
        (json!({"nope": 1}), "Unknown"),
        (json!({"backup": {"nope": 1}}), "Unknown"),
        (json!({"authExemptMinutes": 61}), "Invalid"),
        (json!({"authExemptMinutes": -1}), "Invalid"),
        (json!({"authExemptMinutes": 5.5}), "Invalid"),
        (json!({"rescanIntervalHours": 0}), "Invalid"),
        (json!({"backup": {"keep": "7"}}), "Invalid"),
        (json!({"backup": "all"}), "Invalid"),
        (json!({"closeToTray": "yes"}), "Invalid"),
        (json!({"backup": {"dir": ""}}), "Invalid"),
        (json!({"backup": {"dir": "relative"}}), "Invalid"),
        (
            json!({"backup": {"keep": 5}, "firstRunCompleted": 3}),
            "Invalid",
        ),
    ];

    for (body, expected) in cases {
        // Act
        let err = settings::update(&mut db.conn, &patch(body.clone())).unwrap_err();

        // Assert
        let kind = match err {
            SettingsError::Unknown(_) => "Unknown",
            SettingsError::Invalid(_) => "Invalid",
            SettingsError::Db(_) => "Db",
        };
        assert_eq!(kind, expected, "{body}");
    }
    let (after, _) = settings::load(&db.conn).unwrap();
    assert_eq!(
        after,
        Settings::default(),
        "a rejected patch changes nothing"
    );
}

#[test]
fn t_store_09c_backup_dir_can_be_cleared_with_null() {
    let mut db = migrated_db();
    settings::update(&mut db.conn, &patch(json!({"backup": {"dir": "D:\\B"}}))).unwrap();
    let cleared = settings::update(&mut db.conn, &patch(json!({"backup": {"dir": null}}))).unwrap();
    assert_eq!(cleared.backup.dir, None);
}

fn names(dir: &Path) -> Vec<String> {
    list_backups(dir)
        .unwrap()
        .into_iter()
        .map(|b| b.file_name)
        .collect()
}

#[test]
fn t_store_10_backup_is_valid_and_old_ones_are_pruned_to_keep_count() {
    // Arrange
    let db = migrated_db();
    insert_asset(&db.conn);
    let dir = db.dir.path().join("my-backups");

    // Act: 5개 만들고 3개만 보관
    let mut last = None;
    for i in 0..5 {
        last = Some(backup(&db.db_path, &dir, BackupKind::Manual, 3, T0 + i));
    }

    // Assert
    let last = last.unwrap();
    assert_eq!(last.file_name, format!("mam-{}.sqlite", T0 + 4));
    assert_eq!(last.kind, BackupKind::Manual);
    assert_eq!(last.created_at, T0 + 4);
    assert!(last.size_bytes > 0);
    assert_eq!(
        names(&dir),
        [T0 + 4, T0 + 3, T0 + 2].map(|ms| format!("mam-{ms}.sqlite"))
    );
    let opened = Connection::open(dir.join(&last.file_name)).unwrap();
    let check: String = opened
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    let assets: i64 = opened
        .query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))
        .unwrap();
    assert_eq!(assets, 1);
}

#[test]
fn t_store_10c_contract_serialization_and_kinds_from_file_names() {
    // Arrange
    let db = migrated_db();
    let dir = db.dir.path().join("b");
    std::fs::create_dir_all(&dir).unwrap();
    for name in [
        "mam-100.sqlite",
        "mam-auto-200.sqlite",
        "mam-pre-migrate-v1-300.sqlite",
        "mam-pre-update-v0.1.0-400.sqlite",
        "mam-pre-reset-500.sqlite",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }

    // Act
    let listed = list_backups(&dir).unwrap();
    let json = serde_json::to_value(&listed).unwrap();

    // Assert: 최신 순, snake_case kind, camelCase 필드
    let kinds: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["pre_reset", "pre_migrate", "pre_migrate", "auto", "manual"]
    );
    assert_eq!(
        json[3],
        json!({"fileName": "mam-auto-200.sqlite", "createdAt": 200, "sizeBytes": 1, "kind": "auto"})
    );
}

#[test]
fn t_store_10b_pruning_keeps_protected_and_treats_safety_backups_separately() {
    // Arrange: 마이그레이션·업데이트 직전 백업 5개 + 사용자 파일
    let db = migrated_db();
    let dir = db.dir.path().join("b");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..5 {
        let name = if i % 2 == 0 {
            format!("mam-pre-migrate-v1-{}.sqlite", T0 + i)
        } else {
            format!("mam-pre-update-v0.1.0-{}.sqlite", T0 + i)
        };
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    std::fs::write(dir.join("mam-notes-123.sqlite"), b"mine").unwrap();
    std::fs::write(dir.join("readme.txt"), b"mine").unwrap();
    let protected: PathBuf = dir.join(format!("mam-pre-migrate-v1-{T0}.sqlite"));

    // Act
    create_backup(
        &db.db_path,
        &dir,
        BackupKind::Auto,
        1,
        &[protected],
        &FixedClock(T0 + 100),
    )
    .unwrap();

    // Assert
    let listed = list_backups(&dir).unwrap();
    let safety: Vec<_> = listed
        .iter()
        .filter(|b| b.kind == BackupKind::PreMigrate)
        .collect();
    assert_eq!(
        safety.len(),
        SAFETY_BACKUPS_KEPT + 1,
        "3 newest + 1 protected"
    );
    assert!(
        safety
            .iter()
            .any(|b| b.file_name.ends_with(&format!("-{T0}.sqlite")))
    );
    assert_eq!(
        listed.iter().filter(|b| b.kind == BackupKind::Auto).count(),
        1
    );
    assert!(
        dir.join("mam-notes-123.sqlite").exists(),
        "foreign files are never touched"
    );
    assert!(dir.join("readme.txt").exists());
}

#[test]
fn t_store_11_failed_backup_leaves_no_partial_file_and_reports_error() {
    // Arrange: 복사 도중 디스크가 가득 찬 상황 (부분 파일을 쓴 뒤 실패)
    let db = migrated_db();
    let dir = db.dir.path().join("full-disk");
    let failing_copy = |_src: &Path, dest: &Path| -> Result<(), DbError> {
        std::fs::write(dest, b"half a database").unwrap();
        Err(DbError::Io(std::io::Error::from(
            std::io::ErrorKind::StorageFull,
        )))
    };
    backup(&db.db_path, &dir, BackupKind::Manual, 7, T0);

    // Act
    let err = create_backup_with(
        &db.db_path,
        &dir,
        BackupKind::Manual,
        7,
        &[],
        &FixedClock(T0 + 1),
        &failing_copy,
    )
    .unwrap_err();

    // Assert
    assert!(err.is_disk_full());
    let files: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        files,
        [format!("mam-{T0}.sqlite")],
        "only the earlier good backup remains"
    );
}

#[test]
fn t_store_11b_unusable_backup_dir_or_missing_db_is_an_error_without_leftovers() {
    let db = migrated_db();
    let clock = FixedClock(T0);
    // 백업 폴더 자리에 파일이 있다
    let blocker = db.dir.path().join("blocker");
    std::fs::write(&blocker, b"file").unwrap();
    assert!(create_backup(&db.db_path, &blocker, BackupKind::Manual, 7, &[], &clock).is_err());

    // 원본 DB 가 없다 (백업 API 가 빈 DB 를 만들어 내지 않는다)
    let dir = db.dir.path().join("b2");
    let missing = db.dir.path().join("nope.sqlite");
    assert!(create_backup(&missing, &dir, BackupKind::Manual, 7, &[], &clock).is_err());
    assert!(list_backups(&dir).unwrap().is_empty());
    assert!(!missing.exists());

    // 마이그레이션 직전 백업은 러너만 만든다
    assert!(create_backup(&db.db_path, &dir, BackupKind::PreMigrate, 7, &[], &clock).is_err());
}

#[test]
fn t_store_11c_stale_partial_files_are_cleaned_and_listing_a_missing_dir_is_empty() {
    let db = migrated_db();
    let dir = db.dir.path().join("b");
    assert!(list_backups(&dir.join("absent")).unwrap().is_empty());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("mam-1.sqlite.partial"), b"crashed").unwrap();

    backup(&db.db_path, &dir, BackupKind::Manual, 7, T0);

    assert!(!dir.join("mam-1.sqlite.partial").exists());
    assert_eq!(names(&dir), [format!("mam-{T0}.sqlite")]);
}

#[test]
fn t_prot_13_password_hash_is_hidden_from_settings_and_swaps_compare_and_set() {
    // Arrange
    let mut db = migrated_db();
    assert_eq!(settings::load_password_hash(&db.conn).unwrap(), None);

    // Act
    let first = settings::swap_password_hash(&mut db.conn, None, "$argon2id$one").unwrap();
    let stale = settings::swap_password_hash(&mut db.conn, None, "$argon2id$two").unwrap();
    let changed =
        settings::swap_password_hash(&mut db.conn, Some("$argon2id$one"), "$argon2id$three")
            .unwrap();
    let wrong_expected =
        settings::swap_password_hash(&mut db.conn, Some("$argon2id$one"), "$argon2id$four")
            .unwrap();

    // Assert: 경합한 쪽은 거부, 설정 계층(load/update)에는 나타나지 않는다
    assert!(first && changed);
    assert!(!stale && !wrong_expected);
    assert_eq!(
        settings::load_password_hash(&db.conn).unwrap().as_deref(),
        Some("$argon2id$three")
    );
    let (loaded, warnings) = settings::load(&db.conn).unwrap();
    assert_eq!(loaded, Settings::default());
    assert!(warnings.is_empty());
    assert!(!serde_json::to_string(&loaded).unwrap().contains("argon2"));
    let err = settings::update(&mut db.conn, &patch(json!({"password_hash": "x"}))).unwrap_err();
    assert!(matches!(err, SettingsError::Unknown(_)));
}
