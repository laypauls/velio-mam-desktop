//! 일관된 스냅샷 생성(`snapshot`)과 백업에서의 복원(`restore_backup`).
//!
//! 둘 다 SQLite 백업 API 를 쓴다 — 실행 중 DB 파일 단순 복사 금지(WAL 미반영).
//! 복원은 새 파일을 옆에 만들어 검증한 뒤 교체하므로, 실패하면 대상 DB 는 그대로다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, MAIN_DB, OpenFlags};

use crate::error::DbError;

/// `db_path` 의 일관된 스냅샷을 `dest` 에 만든다. `dest` 가 이미 있으면 `BackupExists`.
pub fn snapshot(db_path: &Path, dest: &Path) -> Result<(), DbError> {
    if dest.exists() {
        return Err(DbError::BackupExists(dest.to_path_buf()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let result = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .and_then(|conn| conn.backup(MAIN_DB, dest, None));
    if let Err(e) = result {
        let _ = std::fs::remove_file(dest);
        return Err(e.into());
    }
    Ok(())
}

/// `backup` 파일의 내용으로 `target` DB 를 교체한다.
///
/// 순서: 백업 검증 -> 백업 API 로 `<target>.restore-tmp` 생성·검증 -> 대상 WAL 체크포인트·`-wal`/`-shm` 제거
/// -> 교체. 없거나 손상된 백업, 대상이 다른 연결에 열려 있는 경우는 오류이며 대상은 변하지 않는다.
/// 호출 전에 대상에 대한 모든 연결을 닫아야 한다.
pub fn restore_backup(backup: &Path, target: &Path) -> Result<(), DbError> {
    if !backup.is_file() {
        return Err(invalid(backup, "backup file does not exist"));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = sibling(target, ".restore-tmp");
    remove_if_exists(&tmp)?;

    let built = build_verified_copy(backup, &tmp);
    if let Err(e) = built {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    let swapped = detach_wal(target).and_then(|()| Ok(std::fs::rename(&tmp, target)?));
    if let Err(e) = swapped {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn build_verified_copy(backup: &Path, tmp: &Path) -> Result<(), DbError> {
    let leftovers = WalLeftovers::capture(backup);
    let copied = copy_via_backup_api(backup, tmp);
    leftovers.clean();
    copied?;

    let copy = Connection::open_with_flags(tmp, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| invalid(tmp, e))?;
    check_integrity(&copy).map_err(|e| invalid(backup, e))?;
    // 닫는 시점에 WAL 이 비워져 tmp 하나만 남는다
    copy.close().map_err(|(_, e)| DbError::from(e))
}

fn copy_via_backup_api(backup: &Path, tmp: &Path) -> Result<(), DbError> {
    let src = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| invalid(backup, e))?;
    check_integrity(&src).map_err(|e| invalid(backup, e))?;
    // 빈 파일은 SQLite 가 "빈 DB"로 열어 무결성 검사를 통과한다 — 복원하면 데이터를 잃으므로 거부
    let objects: i64 = src
        .query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))
        .map_err(|e| invalid(backup, e))?;
    if objects == 0 {
        return Err(invalid(backup, "backup contains no schema"));
    }
    src.backup(MAIN_DB, tmp, None)
        .map_err(|e| invalid(backup, e))
}

fn check_integrity(conn: &Connection) -> Result<(), String> {
    let result: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("integrity_check: {result}"))
    }
}

/// 대상의 WAL 을 본 파일에 반영(체크포인트)한 뒤 `-wal`/`-shm` 을 지운다.
/// 대상이 없거나 열 수 없는 파일이면 체크포인트는 건너뛴다 — 어차피 교체될 파일이다.
fn detach_wal(target: &Path) -> Result<(), DbError> {
    if target.is_file()
        && let Ok(conn) = Connection::open_with_flags(target, OpenFlags::SQLITE_OPEN_READ_WRITE)
    {
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        let _ = conn.close();
    }
    remove_if_exists(&sibling(target, "-wal"))?;
    remove_if_exists(&sibling(target, "-shm"))
}

/// 읽기 전용으로 백업을 열면 SQLite 가 `-wal`/`-shm` 을 만들 수 있다 — 우리가 만든 것만 치운다.
struct WalLeftovers(Vec<PathBuf>);

impl WalLeftovers {
    fn capture(db: &Path) -> Self {
        Self(
            ["-wal", "-shm"]
                .iter()
                .map(|s| sibling(db, s))
                .filter(|p| !p.exists())
                .collect(),
        )
    }

    fn clean(self) {
        for p in self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_if_exists(path: &Path) -> Result<(), DbError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn invalid(path: &Path, reason: impl ToString) -> DbError {
    DbError::BackupInvalid {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

/// [`inspect_backup`] 가 읽어 낸 백업 정보.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupMeta {
    /// 백업에 적용된 마이그레이션 중 가장 높은 버전
    pub schema_version: i64,
}

/// 복원 전에 백업 파일을 읽기 전용으로 검사한다: SQLite DB 인지, `integrity_check` 통과, 스키마가 비어 있지 않은지,
/// 마이그레이션 이력이 있는지. 파일은 바꾸지 않는다(읽기 전용으로 열며 생긴 `-wal`/`-shm` 은 치운다).
/// 스키마 버전이 앱보다 높은지는 호출자가 `latest_version()` 과 비교해 판단한다.
pub fn inspect_backup(backup: &Path) -> Result<BackupMeta, DbError> {
    if !backup.is_file() {
        return Err(invalid(backup, "backup file does not exist"));
    }
    let leftovers = WalLeftovers::capture(backup);
    let result = read_meta(backup);
    leftovers.clean();
    result
}

fn read_meta(backup: &Path) -> Result<BackupMeta, DbError> {
    let conn = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| invalid(backup, e))?;
    check_integrity(&conn).map_err(|e| invalid(backup, e))?;
    let has_history: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'refinery_schema_history'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| invalid(backup, e))?;
    if has_history == 0 {
        return Err(invalid(backup, "backup has no migration history"));
    }
    let version: Option<i64> = conn
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |r| r.get(0),
        )
        .map_err(|e| invalid(backup, e))?;
    version
        .map(|schema_version| BackupMeta { schema_version })
        .ok_or_else(|| invalid(backup, "backup has no applied migrations"))
}

/// 파일 하나에 `PRAGMA quick_check` 를 돌린다 (자동 백업 직후 검사용). 통과하면 `Ok`, 아니면 `BackupInvalid`.
pub fn quick_check(path: &Path) -> Result<(), DbError> {
    let leftovers = WalLeftovers::capture(path);
    let result = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| invalid(path, e))
        .and_then(|conn| {
            let r: String = conn
                .query_row("PRAGMA quick_check", [], |r| r.get(0))
                .map_err(|e| invalid(path, e))?;
            if r == "ok" {
                Ok(())
            } else {
                Err(invalid(path, format!("quick_check: {r}")))
            }
        });
    leftovers.clean();
    result
}
