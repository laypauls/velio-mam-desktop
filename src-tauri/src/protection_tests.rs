// Categories: Happy (비밀번호 흐름, 삭제·이동) / Validation (길이·중복 설정) / Business rule (인증 게이트, 면제 0분, 오류 코드,
// 초기화 순서) / External (휴지통 실패, 백업 실패) / Time (면제 만료·잠금, 주입 시계).
// 실제 SQLite + 실제 스캔 + 가짜 휴지통 — 실제 휴지통은 쓰지 않는다. 픽스처는 임시 디렉터리뿐이며 원본 무변경을 단언한다.
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use mam_core::ingest::{IngestConfig, IngestDeps};
use mam_core::protect::ProtectError;
use mam_core::scan::scan_folder;
use mam_core::storage::DataPaths;
use mam_db::{BackupKind, Clock, DbPool, Priority, migrate, open};

use super::*;
use crate::reset::{self, StartupReset};

const PW: &str = "correct-horse-1";
const T0: i64 = 1_800_000_000_000;

struct FakeClock(AtomicI64);

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 파일을 `bin` 폴더로 옮기는 가짜 휴지통 (호출 기록 포함)
struct FakeTrash {
    bin: PathBuf,
    calls: Mutex<Vec<PathBuf>>,
    fail: AtomicBool,
}

impl Trash for FakeTrash {
    fn trash(&self, path: &Path) -> Result<(), ProtectError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        if self.fail.load(Ordering::SeqCst) {
            return Err(ProtectError::Io("locked".into()));
        }
        std::fs::create_dir_all(&self.bin).unwrap();
        std::fs::rename(path, self.bin.join(path.file_name().unwrap()))
            .map_err(|e| ProtectError::Io(e.to_string()))
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    paths: DataPaths,
    media: PathBuf,
    clock: Arc<FakeClock>,
    trash: Arc<FakeTrash>,
    svc: Protection,
}

fn fixture() -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let paths = DataPaths::new(base.join("MAMData"));
    paths.ensure_layout().unwrap();
    let media = base.join("media");
    std::fs::create_dir_all(media.join("moved")).unwrap();
    std::fs::write(media.join("a.jpg"), b"aaaa").unwrap();
    std::fs::write(media.join("b.jpg"), b"bbbbbb").unwrap();
    let clock = Arc::new(FakeClock(AtomicI64::new(T0)));
    let mut conn = open(&paths.db()).unwrap();
    migrate(&mut conn, &paths.backups(), &*clock).unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&paths.db(), 2).unwrap());
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&pool),
            clock: clock.clone(),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![paths.root().to_path_buf()],
            ..IngestConfig::default()
        },
    );
    let view = ingest.add_folder(media.to_str().unwrap()).unwrap();
    scan_folder(
        ingest.env(),
        view.folder_id,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap();
    let storage = Arc::new(Storage {
        paths: paths.clone(),
        pool,
        bootstrap_dir: base.join("boot"),
        _lock: None,
    });
    let trash = Arc::new(FakeTrash {
        bin: base.join("bin"),
        calls: Mutex::default(),
        fail: AtomicBool::new(false),
    });
    let deps = OriginalsDeps {
        trash: trash.clone(),
        volumes: Arc::new(WindowsVolumes),
        identity: Arc::new(WindowsFileIdentity),
        hasher: Arc::new(Blake3Hasher),
    };
    let svc = Protection::new(storage, ingest, clock.clone(), deps);
    Fx {
        _dir: dir,
        paths,
        media,
        clock,
        trash,
        svc,
    }
}

impl Fx {
    fn pw(s: &str) -> Zeroizing<String> {
        Zeroizing::new(s.to_owned())
    }

    fn set_and_verify(&self) {
        self.svc.set_password(&Self::pw(PW)).unwrap();
        self.svc.verify_password(&Self::pw(PW)).unwrap();
    }

    fn asset_id(&self, file: &str) -> i64 {
        self.svc
            .storage
            .pool
            .read(|c| {
                Ok(c.query_row(
                    "SELECT asset_id FROM file_locations WHERE rel_path LIKE ?1",
                    [format!("%{file}")],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    fn status_of(&self, id: i64) -> String {
        self.svc
            .storage
            .pool
            .read(|c| {
                Ok(c.query_row(
                    "SELECT status FROM media_assets WHERE asset_id = ?1",
                    [id],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    fn code<T: std::fmt::Debug>(r: Result<T, CommandError>) -> ErrorCode {
        r.unwrap_err().code()
    }

    fn calls(&self) -> usize {
        self.trash.calls.lock().unwrap().len()
    }
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn t_prot_16_password_flow_validates_stores_a_hash_only_and_locks_after_five_failures() {
    // Arrange
    let fx = fixture();
    assert!(!fx.svc.status().unwrap().password_set);

    // Act + Assert: 검증
    assert_eq!(
        Fx::code(fx.svc.set_password(&Fx::pw("short"))),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        Fx::code(
            fx.svc
                .change_password(&Fx::pw(PW), &Fx::pw("new-password-1"))
        ),
        ErrorCode::PasswordNotSet
    );
    assert_eq!(
        Fx::code(fx.svc.verify_password(&Fx::pw(PW))),
        ErrorCode::PasswordNotSet
    );
    // 설정 -> 면제는 시작하지 않는다
    let status = fx.svc.set_password(&Fx::pw(PW)).unwrap();
    assert!(status.password_set && status.auth_valid_until.is_none());
    assert_eq!(
        Fx::code(fx.svc.set_password(&Fx::pw("another-pass-1"))),
        ErrorCode::PasswordAlreadySet
    );
    // 저장소에는 PHC 해시만 있고, 설정 조회 결과에는 나타나지 않는다
    let stored = fx.svc.stored_hash().unwrap().unwrap();
    assert!(stored.starts_with("$argon2id$") && !stored.contains(PW));
    let settings_json = serde_json::to_string(&fx.svc.storage.get_settings().unwrap()).unwrap();
    assert!(!settings_json.contains("argon2") && !settings_json.contains(PW));

    // 변경: 현재 비밀번호가 틀리면 거부, 맞으면 새 비밀번호로 검증된다
    assert_eq!(
        Fx::code(
            fx.svc
                .change_password(&Fx::pw("wrong-password"), &Fx::pw("new-password-1"))
        ),
        ErrorCode::PasswordIncorrect
    );
    fx.svc
        .change_password(&Fx::pw(PW), &Fx::pw("new-password-1"))
        .unwrap();
    assert_eq!(
        Fx::code(fx.svc.verify_password(&Fx::pw(PW))),
        ErrorCode::PasswordIncorrect
    );

    // 잠금: 변경 성공이 실패 횟수를 0으로 돌렸고 위 검증 실패가 1회 -> 4회 더 실패하면 5회째 이후 맞는 비밀번호도 거부
    for _ in 0..4 {
        let _ = fx.svc.verify_password(&Fx::pw("wrong-password"));
    }
    assert_eq!(
        Fx::code(fx.svc.verify_password(&Fx::pw("new-password-1"))),
        ErrorCode::TooManyAttempts
    );
    fx.clock.0.fetch_add(30_000, Ordering::SeqCst);
    let ok = fx.svc.verify_password(&Fx::pw("new-password-1")).unwrap();
    assert_eq!(ok.auth_valid_until, Some(T0 + 30_000 + 5 * 60_000));
}

#[test]
fn t_prot_17_protected_commands_are_gated_and_leave_originals_untouched() {
    // Arrange
    let fx = fixture();
    let id = fx.asset_id("a.jpg");
    let before = snapshot(&fx.media);

    // Act + Assert: 비밀번호 미설정 -> password_not_set
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[id])),
        ErrorCode::PasswordNotSet
    );
    // 설정만 하고 인증 전 -> auth_required (삭제·이동 모두)
    fx.svc.set_password(&Fx::pw(PW)).unwrap();
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[id])),
        ErrorCode::AuthRequired
    );
    let dest = fx.media.join("moved");
    assert_eq!(
        Fx::code(fx.svc.move_originals(&[id], dest.to_str().unwrap())),
        ErrorCode::AuthRequired
    );
    assert_eq!((fx.calls(), snapshot(&fx.media)), (0, before.clone()));

    // 면제 만료 (주입 시계)
    fx.svc.verify_password(&Fx::pw(PW)).unwrap();
    fx.clock.0.fetch_add(5 * 60_000, Ordering::SeqCst);
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[id])),
        ErrorCode::AuthRequired
    );

    // 입력 검증: 빈 목록·빈 대상·존재하지 않는 대상 폴더
    fx.svc.verify_password(&Fx::pw(PW)).unwrap();
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[])),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        Fx::code(fx.svc.move_originals(&[id], "")),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        Fx::code(fx.svc.move_originals(&[id], r"Z:\no\such\dir")),
        ErrorCode::InvalidInput
    );
    assert_eq!(fx.calls(), 0);
    assert_eq!(snapshot(&fx.media), before, "originals never changed");
}

#[test]
fn t_prot_18_delete_goes_through_the_bin_reports_per_asset_failures_and_spares_the_rest() {
    // Arrange
    let fx = fixture();
    fx.set_and_verify();
    let (a, b) = (fx.asset_id("a.jpg"), fx.asset_id("b.jpg"));

    // Act: 존재하지 않는 자산이 섞여도 나머지는 처리
    let r = fx.svc.delete_originals(&[a, 9_999_999]).unwrap();

    // Assert
    assert_eq!(r.succeeded, 1);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].asset_id, 9_999_999);
    assert_eq!(fx.status_of(a), "deleted");
    assert_eq!(fx.status_of(b), "active");
    assert!(!fx.media.join("a.jpg").exists() && fx.trash.bin.join("a.jpg").exists());
    assert_eq!(std::fs::read(fx.media.join("b.jpg")).unwrap(), b"bbbbbb");

    // 휴지통 실패(잠긴 파일) -> 자산별 실패, 자산은 그대로
    fx.trash.fail.store(true, Ordering::SeqCst);
    let r = fx.svc.delete_originals(&[b]).unwrap();
    assert_eq!((r.succeeded, r.failed.len()), (0, 1));
    assert_eq!(fx.status_of(b), "active");
    assert!(fx.media.join("b.jpg").exists());
}

#[test]
fn t_prot_19_move_relinks_the_asset_and_zero_minute_exemption_allows_exactly_one_command() {
    // Arrange: 면제 0분 (매번 인증)
    let fx = fixture();
    fx.svc
        .storage
        .update_settings(
            serde_json::json!({"authExemptMinutes": 0})
                .as_object()
                .unwrap()
                .clone(),
            &mam_core::storage::SystemDriveProbe,
            &[],
        )
        .unwrap();
    fx.svc.set_password(&Fx::pw(PW)).unwrap();
    let id = fx.asset_id("a.jpg");
    let dest = fx.media.join("moved");

    // Act: 인증 직후 한 번은 실행되고
    let status = fx.svc.verify_password(&Fx::pw(PW)).unwrap();
    let moved = fx
        .svc
        .move_originals(&[id], dest.to_str().unwrap())
        .unwrap();

    // Assert
    assert_eq!(status.auth_valid_until, None, "UI must prompt every time");
    assert_eq!((moved.succeeded, moved.failed.len()), (1, 0), "{moved:?}");
    assert!(dest.join("a.jpg").exists() && !fx.media.join("a.jpg").exists());
    assert_eq!(fx.status_of(id), "active", "asset kept and relinked");
    // 두 번째는 다시 인증이 필요하다
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[id])),
        ErrorCode::AuthRequired
    );
}

#[test]
fn t_prot_20_protect_errors_map_to_contract_codes_without_leaking_details() {
    let secret = r"C:\Users\someone\secret-folder";
    let cases: Vec<(ProtectError, ErrorCode)> = vec![
        (
            ProtectError::InvalidInput(secret.into()),
            ErrorCode::InvalidInput,
        ),
        (ProtectError::PasswordNotSet, ErrorCode::PasswordNotSet),
        (
            ProtectError::PasswordAlreadySet,
            ErrorCode::PasswordAlreadySet,
        ),
        (
            ProtectError::PasswordIncorrect,
            ErrorCode::PasswordIncorrect,
        ),
        (ProtectError::AuthRequired, ErrorCode::AuthRequired),
        (
            ProtectError::TooManyAttempts { retry_after_ms: 1 },
            ErrorCode::TooManyAttempts,
        ),
        (ProtectError::NotFound(secret.into()), ErrorCode::NotFound),
        (ProtectError::Offline(secret.into()), ErrorCode::Offline),
        (ProtectError::Io(secret.into()), ErrorCode::Io),
        (ProtectError::BackupFailed(secret.into()), ErrorCode::Io),
        (ProtectError::Internal(secret.into()), ErrorCode::Internal),
    ];
    for (err, code) in cases {
        let mapped: CommandError = err.into();
        assert_eq!(mapped.code(), code);
        let json = serde_json::to_string(&mapped).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("Users"),
            "{json}"
        );
    }
}

#[test]
fn t_prot_21_reset_requires_the_password_makes_a_backup_first_and_applies_before_the_db_opens() {
    // Arrange
    let fx = fixture();
    fx.set_and_verify();
    let media_before = snapshot(&fx.media);
    let paths = fx.paths.clone();

    // Act + Assert: 비밀번호 없음/틀림 -> 거부, 백업·표지 없음
    assert_eq!(
        Fx::code(fx.svc.prepare_reset(None, false)),
        ErrorCode::AuthRequired
    );
    assert_eq!(
        Fx::code(fx.svc.prepare_reset(Some(&Fx::pw("wrong-password")), false)),
        ErrorCode::PasswordIncorrect
    );
    assert!(
        mam_db::list_backups(&paths.backups())
            .unwrap()
            .iter()
            .all(|b| b.kind != BackupKind::PreReset)
    );
    assert!(!reset::is_pending(&paths));

    // 올바른 비밀번호 -> 백업 -> 표지 (데이터는 아직 그대로)
    fx.svc.prepare_reset(Some(&Fx::pw(PW)), false).unwrap();
    let backups = mam_db::list_backups(&paths.backups()).unwrap();
    let pre = backups
        .iter()
        .find(|b| b.kind == BackupKind::PreReset)
        .unwrap();
    assert!(reset::is_pending(&paths) && paths.db().is_file());
    let backup_bytes = std::fs::read(paths.backups().join(&pre.file_name)).unwrap();

    // 재시작 흉내: 모든 연결을 닫고 시작 시 적용 (DB 를 열기 전)
    drop(fx.svc);
    let outcome = reset::apply_pending(&paths).unwrap();

    // Assert: DB 는 사라지고(아직 안 열렸다), 백업·원본은 그대로
    assert!(matches!(outcome, StartupReset::Applied(_)));
    assert!(!paths.db().exists());
    assert!(paths.database_dir().is_dir() && paths.backups().is_dir());
    assert!(!reset::is_pending(&paths));
    assert_eq!(
        std::fs::read(paths.backups().join(&pre.file_name)).unwrap(),
        backup_bytes
    );
    assert_eq!(snapshot(&fx.media), media_before);
    assert_eq!(
        reset::apply_pending(&paths).unwrap(),
        StartupReset::None,
        "idempotent"
    );
}

#[test]
fn t_prot_22_forgot_password_resets_without_auth_and_a_failed_backup_aborts_untouched() {
    // Arrange
    let fx = fixture();
    fx.svc.set_password(&Fx::pw(PW)).unwrap();
    let blocker = fx.media.parent().unwrap().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    fx.svc
        .storage
        .update_settings(
            serde_json::json!({"backup": {"dir": blocker.to_str().unwrap()}})
                .as_object()
                .unwrap()
                .clone(),
            &mam_core::storage::SystemDriveProbe,
            &[],
        )
        .unwrap();

    // Act: 백업 폴더가 파일이라 백업 실패 -> 분실 경로여도 중단
    let err = fx.svc.prepare_reset(None, true).unwrap_err();

    // Assert
    assert_eq!(err.code(), ErrorCode::Io);
    assert!(!reset::is_pending(&fx.paths));
    assert!(fx.paths.db().is_file());
    assert!(fx.svc.status().unwrap().password_set);

    // 백업 폴더를 정상으로 되돌리면 분실 경로는 인증 없이 통과
    fx.svc
        .storage
        .pool
        .write(Priority::High, |c| {
            c.execute("DELETE FROM settings WHERE key = 'backup_dir'", [])?;
            Ok(())
        })
        .unwrap();
    fx.svc.prepare_reset(None, true).unwrap();
    assert!(reset::is_pending(&fx.paths));
}

#[test]
fn t_prot_23_pending_reset_with_a_vanished_backup_is_cancelled_and_deletes_nothing() {
    // Arrange: 표지는 있는데 백업 파일이 사라졌다
    let fx = fixture();
    let paths = fx.paths.clone();
    let ghost = paths.backups().join("mam-pre-reset-1.sqlite");
    std::fs::write(&ghost, b"b").unwrap();
    let plan = mam_core::protect::plan_reset(
        &paths,
        &mam_core::protect::BackupOutcome::Created(ghost.clone()),
        &[],
    )
    .unwrap();
    mam_core::protect::write_reset_marker(&paths, &*fx.clock, &plan).unwrap();
    std::fs::remove_file(&ghost).unwrap();
    drop(fx.svc);

    // Act
    let outcome = reset::apply_pending(&paths).unwrap();

    // Assert: 아무것도 지우지 않고 표지만 버린다 (앱이 영영 못 뜨는 일 방지)
    assert!(matches!(outcome, StartupReset::Cancelled(_)));
    assert!(paths.db().is_file() && !reset::is_pending(&paths));
}

// --- 사용자 지정 경로가 MAM 산출물 위치가 될 때의 겹침 거부 (D1~D4) · 인증 면제 시간 상향 (D7) ---
// Categories: Business rule (겹침 양방향, 고정 하위 폴더, 면제 상향) / Validation / Boundary (면제 0분 1회 허용)

fn upd(
    fx: &Fx,
    v: serde_json::Value,
) -> Result<crate::storage_commands::SettingsView, CommandError> {
    fx.svc.update_settings(
        v.as_object().unwrap().clone(),
        &mam_core::storage::SystemDriveProbe,
    )
}

#[test]
fn t_prot_24_data_dir_overlapping_a_managed_folder_is_rejected_both_directions_and_nothing_is_written()
 {
    // Arrange: 관리 폴더 media — 과거에는 media 자체나 그 상위를 dataDir 로 받아 media\cache 등이 생겼다
    let fx = fixture();
    let boot = &fx.svc.storage.bootstrap_dir;
    let sub = fx.media.join("moved");
    let parent = fx.media.parent().unwrap().to_path_buf();
    let elsewhere = fx._dir.path().join("elsewhere");

    // Act / Assert: 관리 폴더 자체·하위 폴더 -> 겹침 (고른 폴더 + MAMData 가 관리 폴더 안)
    for bad in [&fx.media, &sub] {
        let err = upd(&fx, serde_json::json!({"dataDir": bad.to_str().unwrap()})).unwrap_err();
        assert_eq!(err.code(), ErrorCode::NestedFolder, "{bad:?}");
    }
    assert!(!boot.join("location.json").exists(), "nothing recorded");
    assert!(
        !fx.media.join("MAMData").exists(),
        "no MAM folder inside the originals"
    );

    // 관리 폴더의 상위를 고르면 MAMData 가 형제로 붙는다 -> 겹치지 않아 허용, 최종 경로를 돌려준다 (D1·D3)
    let ok = upd(
        &fx,
        serde_json::json!({"dataDir": parent.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(PathBuf::from(&ok.data_dir), parent.join("MAMData"));

    // 일반 폴더도 고정 하위 폴더가 붙는다
    let ok = upd(
        &fx,
        serde_json::json!({"dataDir": elsewhere.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(PathBuf::from(&ok.data_dir), elsewhere.join("MAMData"));
    assert!(elsewhere.join("MAMData").is_dir());
}

#[test]
fn t_prot_25_backup_dir_overlapping_managed_folders_or_mamdata_internals_is_rejected() {
    // Arrange
    let fx = fixture();
    let parent = fx.media.parent().unwrap().to_path_buf(); // 관리 폴더의 상위 (MAMData 도 포함)
    let internals = [
        fx.paths.cache(),
        fx.paths.database_dir(),
        fx.paths.root().join("indexes"),
        fx.paths.root().to_path_buf(),
    ];

    // Act / Assert
    for bad in [&fx.media, &fx.media.join("moved"), &parent]
        .into_iter()
        .chain(internals.iter())
    {
        let err = upd(
            &fx,
            serde_json::json!({"backup": {"dir": bad.to_str().unwrap()}}),
        )
        .unwrap_err();
        assert_eq!(err.code(), ErrorCode::NestedFolder, "{bad:?}");
    }
    assert_eq!(
        fx.svc.storage.get_settings().unwrap().settings.backup.dir,
        None
    );

    // MAMData 안의 backups 와 외부 폴더는 허용, null 은 기본 위치로 되돌림
    let own = fx.paths.backups();
    upd(
        &fx,
        serde_json::json!({"backup": {"dir": own.to_str().unwrap()}}),
    )
    .unwrap();
    let out = fx._dir.path().join("my-backups");
    upd(
        &fx,
        serde_json::json!({"backup": {"dir": out.to_str().unwrap()}}),
    )
    .unwrap();
    upd(&fx, serde_json::json!({"backup": {"dir": null}})).unwrap();
}

#[test]
fn t_prot_26_data_dir_change_is_checked_against_the_current_custom_backup_dir() {
    // Arrange: 백업 폴더가 외부에 있고, 새 MAMData 가 그 안으로 들어오려 한다
    let fx = fixture();
    let out = fx._dir.path().join("my-backups");
    upd(
        &fx,
        serde_json::json!({"backup": {"dir": out.to_str().unwrap()}}),
    )
    .unwrap();

    // Act: 백업 폴더의 상위를 고르면 MAMData 가 형제 -> OK / 백업 폴더 자체를 고르면 out\MAMData\database 가 백업 폴더 안
    let err = upd(&fx, serde_json::json!({"dataDir": out.to_str().unwrap()})).unwrap_err();

    // Assert
    assert_eq!(err.code(), ErrorCode::NestedFolder);
}

#[test]
fn t_prot_27_add_folder_overlapping_mamdata_is_rejected_in_both_directions() {
    // Arrange: MAMData 는 fixture 의 auto_excludes
    let fx = fixture();
    let base = fx.media.parent().unwrap().to_path_buf();

    // Act
    let parent = fx
        .svc
        .ingest
        .add_folder(base.to_str().unwrap())
        .unwrap_err();
    let inside = fx
        .svc
        .ingest
        .add_folder(fx.paths.cache().to_str().unwrap())
        .unwrap_err();

    // Assert: 상위(MAMData 를 포함)·하위 모두 nested_folder, 메시지에 절대 경로가 없다
    for e in [parent, inside] {
        let c = CommandError::from(e);
        assert_eq!(c.code(), ErrorCode::NestedFolder);
        let json = serde_json::to_string(&c).unwrap();
        assert!(
            !json.contains("MAMData") && !json.contains("\\\\"),
            "{json}"
        );
    }
}

#[test]
fn t_prot_28_reset_protects_managed_folders_and_the_custom_backup_dir_in_the_sidecar() {
    // Arrange: D4 회귀 고정 — 사이드카에 관리 폴더 루트와 사용자 지정 백업 폴더가 있어야 한다
    let fx = fixture();
    let out = fx._dir.path().join("my-backups");
    upd(
        &fx,
        serde_json::json!({"backup": {"dir": out.to_str().unwrap()}}),
    )
    .unwrap();

    // Act
    fx.svc.prepare_reset(None, true).unwrap();

    // Assert
    let raw = std::fs::read(fx.paths.root().join("reset-protected.json")).unwrap();
    let list: Vec<PathBuf> = serde_json::from_slice(&raw).unwrap();
    assert!(
        list.iter()
            .any(|p| mam_core::storage::paths_overlap(p, &fx.media)),
        "{list:?}"
    );
    assert!(
        list.iter()
            .any(|p| mam_core::storage::paths_overlap(p, &out)),
        "{list:?}"
    );
}

#[test]
fn t_prot_29_raising_the_exemption_needs_auth_lowering_does_not_and_no_password_means_free() {
    // Arrange
    let fx = fixture();
    upd(&fx, serde_json::json!({"authExemptMinutes": 5})).unwrap(); // 비밀번호 없음 -> 자유
    fx.svc.set_password(&Fx::pw(PW)).unwrap();

    // Act / Assert: 인증 전 상향 거부, 값 불변
    let err = upd(&fx, serde_json::json!({"authExemptMinutes": 60})).unwrap_err();
    assert_eq!(err.code(), ErrorCode::AuthRequired);
    assert_eq!(
        fx.svc
            .storage
            .get_settings()
            .unwrap()
            .settings
            .auth_exempt_minutes,
        5
    );
    // 같은 값·하향은 항상 허용
    upd(&fx, serde_json::json!({"authExemptMinutes": 5})).unwrap();
    upd(&fx, serde_json::json!({"authExemptMinutes": 3})).unwrap();
    // 인증 면제 안에서는 상향 가능
    fx.svc.verify_password(&Fx::pw(PW)).unwrap();
    let up = upd(&fx, serde_json::json!({"authExemptMinutes": 30})).unwrap();
    assert_eq!(up.settings.auth_exempt_minutes, 30);
}

#[test]
fn t_prot_30_raising_from_zero_consumes_the_single_use_grant_so_a_delete_needs_a_new_verify() {
    // Arrange: 면제 0분 -> verify 는 1회 허용만 준다
    let fx = fixture();
    upd(&fx, serde_json::json!({"authExemptMinutes": 0})).unwrap();
    fx.svc.set_password(&Fx::pw(PW)).unwrap();
    let id = fx.asset_id("a.jpg");
    assert_eq!(
        Fx::code(upd(&fx, serde_json::json!({"authExemptMinutes": 10}))),
        ErrorCode::AuthRequired,
        "no grant yet"
    );

    // Act: verify(1회) -> 상향
    fx.svc.verify_password(&Fx::pw(PW)).unwrap();
    let up = upd(&fx, serde_json::json!({"authExemptMinutes": 10})).unwrap();

    // Assert: 상향은 성공했고, 그 허용으로 삭제를 재사용할 수 없다
    assert_eq!(up.settings.auth_exempt_minutes, 10);
    assert_eq!(
        Fx::code(fx.svc.delete_originals(&[id])),
        ErrorCode::AuthRequired
    );
    assert!(fx.media.join("a.jpg").exists());
}
