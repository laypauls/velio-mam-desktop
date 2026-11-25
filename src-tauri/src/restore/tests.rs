// Categories: Happy (01 복원·보호 유지·알림 1회) / Validation (02 없는/빈/손상/새 스키마/경로형 이름) /
// Business rule (03 인증, 04 동의 철회 쪽, 07 ID 재사용 방지, 09 선택한 안전 백업 보호) /
// External + Concurrency (05 입출력 실패 재시도·한도, 06 사라진 백업·손상 표지) / 파생 인덱스 재구축 요청 (08).
// 실제 SQLite 임시 파일 + 주입 시계. 원본 픽스처 불변 단언 포함.
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::protect::Zeroizing;
use mam_core::storage::{DataPaths, SystemDriveProbe};
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{BackupInfo, BackupKind, DbPool};

use super::notice::{self, StartupNotice};
use super::test_support::*;
use super::{StartupRestore, apply_pending, is_pending, marker_path};
use crate::error::ErrorCode;
use crate::protection::{OriginalsDeps, Protection};
use crate::startup::Boot;
use crate::storage_commands::Storage;

const PW: &str = "correct-horse-1";

fn pw(s: &str) -> Zeroizing<String> {
    Zeroizing::new(s.to_owned())
}

struct Fx {
    dir: tempfile::TempDir,
    paths: DataPaths,
    clock: Arc<TestClock>,
    svc: Protection,
}

fn fixture() -> Fx {
    let (dir, paths) = data_dir();
    let clock = Arc::new(TestClock::new());
    let pool = Arc::new(DbPool::open(&paths.db(), 2).unwrap());
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&pool),
            clock: clock.clone(),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig::default(),
    );
    let storage = Arc::new(Storage {
        paths: paths.clone(),
        pool,
        bootstrap_dir: dir.path().join("boot"),
        _lock: None,
    });
    let svc = Protection::new(storage, ingest, clock.clone(), OriginalsDeps::system());
    Fx {
        dir,
        paths,
        clock,
        svc,
    }
}

impl Fx {
    fn backup(&self) -> BackupInfo {
        self.clock.0.fetch_add(1000, Ordering::SeqCst);
        self.svc.storage.backup_now(&*self.clock).unwrap()
    }

    fn code(&self, name: &str, password: Option<&str>) -> ErrorCode {
        let p = password.map(pw);
        self.svc
            .prepare_restore(name, p.as_ref())
            .unwrap_err()
            .code()
    }

    fn no_side_effects(&self) {
        assert!(!is_pending(&self.paths));
        assert!(
            mam_db::list_backups(&self.paths.backups())
                .unwrap()
                .iter()
                .all(|b| b.kind != BackupKind::PreRestore)
        );
    }

    /// 재시작 흉내: 모든 연결을 닫는다
    fn restart(self) -> (tempfile::TempDir, DataPaths) {
        let Fx {
            dir, paths, svc, ..
        } = self;
        drop(svc);
        (dir, paths)
    }
}

fn conn_at(path: &std::path::Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path).unwrap()
}

#[test]
fn t_restore_01_restores_backup_content_keeps_protection_and_notice_is_read_once() {
    // Arrange: 자산 1 -> 백업 -> 자산 3 으로 늘림 -> 그 뒤 비밀번호·면제 시간 설정, 원본 픽스처
    let fx = fixture();
    let media = fx.dir.path().join("media.jpg");
    std::fs::write(&media, b"original bytes").unwrap();
    let media_mtime = std::fs::metadata(&media).unwrap().modified().unwrap();
    insert_asset(&conn(&fx.paths));
    let info = fx.backup();
    for _ in 0..2 {
        insert_asset(&conn(&fx.paths));
    }
    fx.svc.set_password(&pw(PW)).unwrap();
    let patch = serde_json::json!({"authExemptMinutes": 17});
    fx.svc
        .storage
        .update_settings(patch.as_object().unwrap().clone(), &SystemDriveProbe, &[])
        .unwrap();
    let hash_before = hash(&fx.paths).unwrap();

    // Act: 요청 -> 재시작 -> 기동
    fx.svc
        .prepare_restore(&info.file_name, Some(&pw(PW)))
        .unwrap();
    let marked = is_pending(&fx.paths);
    let (_dir, paths) = fx.restart();
    assert_eq!(
        count(&paths, "SELECT COUNT(*) FROM media_assets"),
        3,
        "untouched until restart"
    );
    let booted = run_boot(&paths, &TestClock::new());

    // Assert
    assert!(marked && booted == Boot::Run && !is_pending(&paths));
    assert_eq!(
        count(&paths, "SELECT COUNT(*) FROM media_assets"),
        1,
        "backup content"
    );
    assert_eq!(
        hash(&paths),
        Some(hash_before),
        "current password hash kept (PR1)"
    );
    assert_eq!(
        count(
            &paths,
            "SELECT CAST(value AS INTEGER) FROM settings WHERE key = 'auth_exempt_minutes'"
        ),
        17
    );
    assert!(
        mam_db::list_backups(&paths.backups())
            .unwrap()
            .iter()
            .any(|b| b.kind == BackupKind::PreRestore),
        "pre_restore backup of the replaced DB"
    );
    assert_eq!(
        notice::take(&paths),
        Some(StartupNotice::Restored {
            backup_file: info.file_name,
            created_at: info.created_at
        })
    );
    assert_eq!(notice::take(&paths), None, "read-once");
    assert_eq!(std::fs::read(&media).unwrap(), b"original bytes");
    assert_eq!(
        std::fs::metadata(&media).unwrap().modified().unwrap(),
        media_mtime
    );
}

#[test]
fn t_restore_02_unusable_backups_are_rejected_and_nothing_changes() {
    // Arrange
    let fx = fixture();
    insert_asset(&conn(&fx.paths));
    let good = fx.backup();
    let dir = fx.paths.backups();
    std::fs::write(dir.join("mam-111.sqlite"), b"").unwrap();
    std::fs::write(dir.join("mam-222.sqlite"), vec![0xAB; 8192]).unwrap();
    let future = dir.join("mam-333.sqlite");
    std::fs::copy(dir.join(&good.file_name), &future).unwrap();
    conn_at(&future)
        .execute(
            "INSERT INTO refinery_schema_history (version, name, applied_on, checksum)
             VALUES (?1, 'future', '2030-01-01T00:00:00Z', '0')",
            [mam_db::latest_version() + 1],
        )
        .unwrap();
    let db_before = std::fs::read(fx.paths.db()).unwrap();

    // Act + Assert
    assert_eq!(
        fx.code("mam-111.sqlite", None),
        ErrorCode::BackupInvalid,
        "empty file"
    );
    assert_eq!(
        fx.code("mam-222.sqlite", None),
        ErrorCode::BackupInvalid,
        "garbage bytes"
    );
    assert_eq!(fx.code("mam-333.sqlite", None), ErrorCode::BackupTooNew);
    assert_eq!(
        fx.code("mam-999.sqlite", None),
        ErrorCode::NotFound,
        "unknown name"
    );
    for name in [
        "..\\..\\x.sqlite",
        "../mam-111.sqlite",
        "C:\\mam-111.sqlite",
        "",
    ] {
        assert_eq!(fx.code(name, None), ErrorCode::NotFound, "{name}");
    }
    fx.no_side_effects();
    assert_eq!(
        std::fs::read(fx.paths.db()).unwrap(),
        db_before,
        "target DB unchanged"
    );
}

#[test]
fn t_restore_03_password_is_required_every_time_even_inside_the_exempt_window() {
    // Arrange
    let fx = fixture();
    let info = fx.backup();
    fx.svc.set_password(&pw(PW)).unwrap();
    fx.svc.verify_password(&pw(PW)).unwrap(); // 면제 시간 시작

    // Act + Assert: 면제 중이어도 비밀번호 필수, 틀리면 거부, 부수효과 없음
    assert_eq!(fx.code(&info.file_name, None), ErrorCode::AuthRequired);
    assert_eq!(
        fx.code(&info.file_name, Some("wrong-password")),
        ErrorCode::PasswordIncorrect
    );
    fx.no_side_effects();
    fx.svc
        .prepare_restore(&info.file_name, Some(&pw(PW)))
        .unwrap();
    assert!(is_pending(&fx.paths));
}

#[test]
fn t_restore_04_consent_follows_the_withdrawn_side() {
    // (백업 동의, 현재 동의) -> 복원 후 동의
    for (backup_given, current_given, expected) in [
        (true, false, false),
        (false, true, false),
        (true, true, true),
        (false, false, false),
    ] {
        // Arrange
        let fx = fixture();
        set_consent(&conn(&fx.paths), backup_given, backup_given.then_some(5));
        let info = fx.backup();
        set_consent(&conn(&fx.paths), current_given, current_given.then_some(9));

        // Act
        fx.svc.prepare_restore(&info.file_name, None).unwrap();
        let (_dir, paths) = fx.restart();
        assert_eq!(run_boot(&paths, &TestClock::new()), Boot::Run);

        // Assert
        let (given, _) = consent(&conn(&paths));
        assert_eq!(
            given, expected,
            "backup={backup_given} current={current_given}"
        );
    }
}

#[test]
fn t_restore_05_io_failure_keeps_the_marker_and_the_next_start_gives_the_same_result() {
    // Arrange: 복원 임시 파일 자리에 폴더를 두어 교체 단계가 입출력 오류로 실패하게 한다
    let fx = fixture();
    insert_asset(&conn(&fx.paths));
    let info = fx.backup();
    insert_asset(&conn(&fx.paths));
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let (_dir, paths) = fx.restart();
    let blocker = paths.database_dir().join("mam.sqlite.restore-tmp");
    std::fs::create_dir(&blocker).unwrap();

    // Act 1: 실패 -> 표지 유지, DB 불변, 시도 횟수 기록
    assert!(apply_pending(&paths).is_err());
    let marker: super::RestoreMarker =
        serde_json::from_slice(&std::fs::read(marker_path(&paths)).unwrap()).unwrap();
    assert_eq!(marker.attempts, 1);
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 2);

    // Act 2: 원인을 없애고 다시 -> 적용, 한 번 더 호출하면 아무 일 없음(멱등)
    std::fs::remove_dir(&blocker).unwrap();
    let applied = apply_pending(&paths).unwrap();

    // Assert
    assert_eq!(applied, StartupRestore::Applied(info.file_name));
    assert_eq!(count(&paths, "SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(apply_pending(&paths).unwrap(), StartupRestore::None);
}

#[test]
fn t_restore_05b_gives_up_after_the_attempt_limit_so_the_app_can_start() {
    // Arrange
    let fx = fixture();
    let info = fx.backup();
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let (_dir, paths) = fx.restart();
    std::fs::create_dir(paths.database_dir().join("mam.sqlite.restore-tmp")).unwrap();

    // Act
    let mut failures = 0;
    let last = loop {
        match apply_pending(&paths) {
            Err(_) => failures += 1,
            Ok(other) => break other,
        }
    };

    // Assert
    assert_eq!(failures, super::MAX_ATTEMPTS - 1);
    assert!(matches!(last, StartupRestore::Discarded(_)));
    assert!(!is_pending(&paths));
}

#[test]
fn t_restore_06_vanished_backup_or_corrupt_marker_discards_without_touching_the_db() {
    // Arrange
    let fx = fixture();
    let info = fx.backup();
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let (_dir, paths) = fx.restart();
    let before = std::fs::read(paths.db()).unwrap();

    // Act 1: 백업이 사라졌다
    std::fs::remove_file(paths.backups().join(&info.file_name)).unwrap();
    let vanished = apply_pending(&paths).unwrap();
    // Act 2: 표지가 손상됐다
    std::fs::write(marker_path(&paths), b"{ not json").unwrap();
    let corrupt = apply_pending(&paths).unwrap();

    // Assert
    assert!(matches!(vanished, StartupRestore::Discarded(_)));
    assert!(matches!(corrupt, StartupRestore::Discarded(_)));
    assert!(!is_pending(&paths));
    assert_eq!(std::fs::read(paths.db()).unwrap(), before);
}

#[test]
fn t_restore_07_ids_issued_after_the_backup_are_never_reused() {
    // Arrange: 백업 시점 자산 1건, 그 뒤 3건 더 발급 (AUTOINCREMENT 상한 4)
    let fx = fixture();
    insert_asset(&conn(&fx.paths));
    let info = fx.backup();
    for _ in 0..3 {
        insert_asset(&conn(&fx.paths));
    }
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let (_dir, paths) = fx.restart();

    // Act
    assert_eq!(run_boot(&paths, &TestClock::new()), Boot::Run);
    let next = insert_asset(&conn(&paths));

    // Assert (H1)
    assert!(
        next > 4,
        "new asset_id {next} must exceed the pre-restore maximum 4"
    );
}

#[test]
fn t_restore_08_restore_requests_a_full_rebuild_of_tantivy_and_vector_generations() {
    // Arrange: 벡터 모델 디렉터리에 활성 세대 포인터가 있다
    let fx = fixture();
    let info = fx.backup();
    let gen_dir = fx.paths.root().join("indexes/vectors/semantic/m1/gen");
    std::fs::create_dir_all(gen_dir.join("g1")).unwrap();
    std::fs::write(gen_dir.join("ACTIVE"), "g1").unwrap();
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let (_dir, paths) = fx.restart();

    // Act
    apply_pending(&paths).unwrap();

    // Assert: Tantivy 요청 표지, 벡터 포인터 제거(원시 데이터·옛 세대 디렉터리는 그대로 — 재구축이 정리)
    assert!(mam_search::sync::rebuild_request_path(&paths.tantivy()).is_file());
    assert!(!gen_dir.join("ACTIVE").exists());
    assert!(gen_dir.join("g1").is_dir());
}

#[test]
fn t_restore_09_pre_restore_does_not_evict_the_chosen_safety_backup() {
    // Arrange: 안전 백업 3개(보관 한도)가 이미 있고, 가장 오래된 것을 복원하려 한다
    let fx = fixture();
    let mut names = Vec::new();
    for i in 0..3 {
        let info = mam_db::create_backup(
            &fx.paths.db(),
            &fx.paths.backups(),
            BackupKind::PreReset,
            7,
            &[],
            &TestClock(AtomicI64::new(T0 + i)),
        )
        .unwrap();
        names.push(info.file_name);
    }

    // Act
    fx.svc.prepare_restore(&names[0], None).unwrap();

    // Assert: 고른 백업이 정리에 밀려나지 않았다
    assert!(fx.paths.backups().join(&names[0]).is_file());
}

#[test]
fn t_restore_10_current_api_key_rows_are_kept_and_never_appear_in_the_marker_as_plaintext() {
    // Arrange: 백업 시점에 openai·gemini 키(암호문) -> 이후 openai 삭제, claude 추가
    let fx = fixture();
    let put = |k: &str, v: &str| {
        conn(&fx.paths)
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [k, v],
            )
            .unwrap();
    };
    put("ai_key_openai", "dpapi1:OLD-OPENAI");
    put("ai_key_gemini", "dpapi1:GEMINI");
    let info = fx.backup();
    conn(&fx.paths)
        .execute("DELETE FROM settings WHERE key = 'ai_key_openai'", [])
        .unwrap();
    put("ai_key_claude", "dpapi1:NEW-CLAUDE");
    put("ai_key_gemini", "dpapi1:GEMINI-ROTATED");

    // Act
    fx.svc.prepare_restore(&info.file_name, None).unwrap();
    let marker_text = std::fs::read_to_string(marker_path(&fx.paths)).unwrap();
    let (_dir, paths) = fx.restart();
    assert_eq!(run_boot(&paths, &TestClock::new()), Boot::Run);

    // Assert: 지운 키는 되살아나지 않고, 새 키·현재 값은 유지된다. 표지에는 암호문 행만 있다
    let keys: Vec<(String, String)> = {
        let c = conn(&paths);
        let mut s = c
            .prepare(r"SELECT key, value FROM settings WHERE key LIKE 'ai\_key\_%' ESCAPE '\' ORDER BY key")
            .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        keys,
        [
            ("ai_key_claude".to_owned(), "dpapi1:NEW-CLAUDE".to_owned()),
            (
                "ai_key_gemini".to_owned(),
                "dpapi1:GEMINI-ROTATED".to_owned()
            ),
        ]
    );
    assert!(marker_text.contains("dpapi1:NEW-CLAUDE") && !marker_text.contains("OLD-OPENAI"));
    let debug = format!(
        "{:?}",
        serde_json::from_str::<super::RestoreMarker>(&marker_text).unwrap()
    );
    assert!(
        !debug.contains("dpapi1"),
        "Debug output must not expose key rows: {debug}"
    );
}
