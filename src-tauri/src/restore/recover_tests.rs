// Categories: Happy (01 최신 유효 백업으로 복구·알림·동의 해제) / Boundary (02 백업 0개 -> 빈 DB, 03 힌트 폴더, 06 CORRUPT 페이지) /
// Validation (04 쓸 수 없는 최신 백업 건너뜀) / External (05 H5 손상이 아닌 오류는 DB 를 옮기지 않음, 07 보관 파일 덮어쓰기 금지) /
// Business rule (08 알림 계약, H1 ID 재사용 방지). 실제 SQLite 임시 파일 + 주입 시계.
use mam_core::storage::DataPaths;
use mam_db::{BackupKind, DbError, create_backup};
use rusqlite::ffi;

use super::notice::{self, StartupNotice};
use super::recover::is_corruption;
use super::test_support::*;
use crate::startup::Boot;

fn backup_at(paths: &DataPaths, dir: &std::path::Path, ms: i64) -> mam_db::BackupInfo {
    create_backup(
        &paths.db(),
        dir,
        BackupKind::Auto,
        7,
        &[],
        &TestClock(std::sync::atomic::AtomicI64::new(ms)),
    )
    .unwrap()
}

fn corrupt_files(paths: &DataPaths) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(paths.database_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(".corrupt-"))
        .collect()
}

#[test]
fn t_recover_01_corrupt_db_is_kept_and_the_newest_valid_backup_restored() {
    // Arrange: 자산 2건 + 비밀번호 + 동의 -> 백업 -> 자산 더 늘림 -> 캐시에 큰 ID 의 썸네일 -> 손상
    let (_d, paths) = data_dir();
    {
        let mut c = conn(&paths);
        insert_asset(&c);
        insert_asset(&c);
        set_hash(&mut c, "phc-from-backup");
        set_consent(&c, true, Some(7));
    }
    let info = backup_at(&paths, &paths.backups(), T0 + 5);
    {
        let c = conn(&paths);
        for _ in 0..8 {
            insert_asset(&c);
        }
    }
    let thumb = paths.thumbnails().join("320/09/00");
    std::fs::create_dir_all(&thumb).unwrap();
    std::fs::write(thumb.join("9.webp"), b"w").unwrap();
    overwrite_with_garbage(&paths);

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert: 복구, 손상 파일 보관, 백업 내용, 보호 설정은 백업 값, 동의는 해제
    assert_eq!(booted, Boot::Run);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 2);
    let kept = corrupt_files(&paths);
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0]).unwrap(), vec![0xAB; 16 * 1024]);
    assert_eq!(hash(&paths).as_deref(), Some("phc-from-backup"));
    assert_eq!(consent(&conn(&paths)), (false, None));
    let notice = notice::take(&paths).unwrap();
    assert_eq!(
        notice,
        StartupNotice::Recovered {
            backup_file: Some(info.file_name),
            created_at: Some(info.created_at),
            corrupt_file: kept[0].to_string_lossy().into_owned(),
            consent_reset: true,
        }
    );
    assert_eq!(notice::take(&paths), None, "read-once");
    // H1: 캐시에 남은 썸네일(ID 9) 때문에 새 자산은 9 보다 큰 ID
    let next = insert_asset(&conn(&paths));
    assert!(
        next > 9,
        "new asset_id {next} must not collide with cached asset 9"
    );
    assert!(
        thumb.join("9.webp").is_file(),
        "valid caches are not deleted"
    );
    assert!(mam_search::sync::rebuild_request_path(&paths.tantivy()).is_file());
}

#[test]
fn t_recover_02_without_any_backup_it_starts_empty_and_keeps_the_corrupt_file() {
    // Arrange
    let (_d, paths) = data_dir();
    insert_asset(&conn(&paths));
    let thumb = paths.thumbnails().join("320/01/00");
    std::fs::create_dir_all(&thumb).unwrap();
    std::fs::write(thumb.join("1.webp"), b"orphan").unwrap();
    overwrite_with_garbage(&paths);

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert_eq!(booted, Boot::Run);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 0);
    assert_eq!(corrupt_files(&paths).len(), 1);
    assert!(
        !thumb.join("1.webp").exists(),
        "orphan caches of the lost assets are removed"
    );
    assert!(paths.thumbnails().is_dir());
    assert!(matches!(
        notice::take(&paths),
        Some(StartupNotice::Recovered {
            backup_file: None,
            created_at: None,
            consent_reset: false,
            ..
        })
    ));
}

#[test]
fn t_recover_03_finds_backups_in_the_custom_folder_through_the_location_hint() {
    // Arrange: 기본 backups/ 는 비어 있고, 사용자 지정 폴더에만 백업이 있다 (힌트 파일이 위치를 안다)
    let (d, paths) = data_dir();
    insert_asset(&conn(&paths));
    let custom = d.path().join("MyBackups");
    backup_at(&paths, &custom, T0 + 1);
    for _ in 0..4 {
        insert_asset(&conn(&paths));
    }
    super::hint::record(&paths, &custom);
    overwrite_with_garbage(&paths);

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert_eq!(booted, Boot::Run);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 1);
    assert!(matches!(
        notice::take(&paths),
        Some(StartupNotice::Recovered {
            backup_file: Some(_),
            ..
        })
    ));
}

#[test]
fn t_recover_04_unusable_newer_backups_are_skipped() {
    // Arrange: 유효한 오래된 백업(자산 1), 더 새로운 쓰레기 파일, 그보다 새로운 새-스키마 백업
    let (_d, paths) = data_dir();
    insert_asset(&conn(&paths));
    let old = backup_at(&paths, &paths.backups(), T0 + 1);
    std::fs::write(
        paths.backups().join(format!("mam-auto-{}.sqlite", T0 + 2)),
        vec![7u8; 4096],
    )
    .unwrap();
    let future = paths.backups().join(format!("mam-auto-{}.sqlite", T0 + 3));
    std::fs::copy(paths.backups().join(&old.file_name), &future).unwrap();
    rusqlite::Connection::open(&future)
        .unwrap()
        .execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
             VALUES (?1, 'future', '2030-01-01T00:00:00Z', '0')",
            [mam_db::latest_version() + 1],
        )
        .unwrap();
    for _ in 0..3 {
        insert_asset(&conn(&paths));
    }
    overwrite_with_garbage(&paths);

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert_eq!(booted, Boot::Run);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 1);
    assert!(matches!(
        notice::take(&paths),
        Some(StartupNotice::Recovered { backup_file: Some(f), .. }) if f == old.file_name
    ));
}

#[test]
fn t_recover_05_only_corrupt_and_notadb_count_as_corruption() {
    // Arrange + Act + Assert: SQLite 오류 코드별 판정 (H5)
    let sqlite =
        |code: i32| DbError::Sqlite(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None));
    assert!(is_corruption(&sqlite(ffi::SQLITE_CORRUPT)));
    assert!(is_corruption(&sqlite(ffi::SQLITE_NOTADB)));
    for code in [
        ffi::SQLITE_BUSY,
        ffi::SQLITE_LOCKED,
        ffi::SQLITE_IOERR,
        ffi::SQLITE_CANTOPEN,
        ffi::SQLITE_PERM,
        ffi::SQLITE_READONLY,
        ffi::SQLITE_FULL,
    ] {
        assert!(!is_corruption(&sqlite(code)), "code {code}");
    }
    assert!(!is_corruption(&DbError::SchemaTooNew { db: 99, app: 1 }));
    assert!(!is_corruption(&DbError::Io(std::io::Error::other("x"))));
}

#[test]
fn t_recover_05b_a_schema_too_new_db_is_not_treated_as_corrupt() {
    // Arrange: 새 스키마 DB (정상 파일) — 구버전 앱이 연 상황
    let (_d, paths) = data_dir();
    conn(&paths)
        .execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
             VALUES (?1, 'future', '2030-01-01T00:00:00Z', '0')",
            [mam_db::latest_version() + 1],
        )
        .unwrap();
    let before = std::fs::read(paths.db()).unwrap();

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert: 기동 실패, DB 는 제자리, 보관 파일·알림 없음
    assert_eq!(booted, Boot::Exit(1));
    assert!(corrupt_files(&paths).is_empty());
    assert_eq!(std::fs::read(paths.db()).unwrap(), before);
    assert_eq!(notice::take(&paths), None);
}

#[test]
fn t_recover_06_malformed_pages_report_sqlite_corrupt_and_recover_too() {
    // Arrange: 헤더(100바이트)는 살리고 첫 페이지 나머지를 망가뜨려 SQLITE_CORRUPT 를 유도한다
    let (_d, paths) = data_dir();
    insert_asset(&conn(&paths));
    backup_at(&paths, &paths.backups(), T0 + 1);
    let mut bytes = std::fs::read(paths.db()).unwrap();
    bytes[100..4096].fill(0xFF);
    std::fs::write(paths.db(), bytes).unwrap();
    let err = mam_db::open(&paths.db())
        .and_then(|mut c| mam_db::migrate(&mut c, &paths.backups(), &TestClock::new()).map(drop))
        .unwrap_err();
    assert!(is_corruption(&err), "{err:?}");

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert_eq!(booted, Boot::Run);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(corrupt_files(&paths).len(), 1);
}

#[test]
fn t_recover_07_an_existing_quarantine_file_is_never_overwritten() {
    // Arrange: 같은 밀리초 이름의 보관 파일이 이미 있다
    let (_d, paths) = data_dir();
    let existing = paths
        .database_dir()
        .join(format!("mam.sqlite.corrupt-{T0}"));
    std::fs::write(&existing, b"previous evidence").unwrap();
    overwrite_with_garbage(&paths);

    // Act
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert_eq!(booted, Boot::Run);
    assert_eq!(std::fs::read(&existing).unwrap(), b"previous evidence");
    assert_eq!(corrupt_files(&paths).len(), 2);
}

#[test]
fn t_recover_08_notice_json_matches_the_frontend_contract() {
    // Arrange + Act + Assert
    assert_eq!(
        serde_json::to_value(StartupNotice::Restored {
            backup_file: "mam-1.sqlite".into(),
            created_at: 5
        })
        .unwrap(),
        serde_json::json!({"kind": "restored", "backupFile": "mam-1.sqlite", "createdAt": 5})
    );
    assert_eq!(
        serde_json::to_value(StartupNotice::Recovered {
            backup_file: None,
            created_at: None,
            corrupt_file: "C:\\x\\mam.sqlite.corrupt-1".into(),
            consent_reset: true
        })
        .unwrap(),
        serde_json::json!({
            "kind": "recovered", "backupFile": null, "createdAt": null,
            "corruptFile": "C:\\x\\mam.sqlite.corrupt-1", "consentReset": true
        })
    );
}

#[test]
fn t_recover_09_hint_is_written_only_when_the_location_changes() {
    // Arrange
    let (d, paths) = data_dir();
    assert_eq!(super::hint::read(&paths), None);

    // Act + Assert
    super::hint::record(&paths, &d.path().join("A"));
    assert_eq!(super::hint::read(&paths), Some(d.path().join("A")));
    super::hint::record(&paths, &d.path().join("B"));
    assert_eq!(super::hint::read(&paths), Some(d.path().join("B")));
    std::fs::write(paths.root().join("backup-location.json"), b"garbage").unwrap();
    assert_eq!(super::hint::read(&paths), None, "corrupt hint is ignored");
}
