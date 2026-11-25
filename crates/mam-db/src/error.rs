use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// DB 에 앱이 모르는 더 새로운 마이그레이션이 적용돼 있다 (구버전 앱이 새 DB 를 연 경우).
    #[error("database schema v{db} is newer than this app supports (v{app})")]
    SchemaTooNew { db: i64, app: i64 },

    /// 이미 적용된 마이그레이션의 내용이 현재 앱의 것과 다르다 (적용된 마이그레이션 수정).
    #[error("applied migration v{version} ({name}) differs from the one shipped with this app")]
    ChecksumMismatch { version: i64, name: String },

    /// 적용 이력의 중간 버전이 앱에 없다 (번호 누락·파일 삭제).
    #[error("applied migration v{version} is not known to this app")]
    UnknownMigration { version: i64 },

    /// 앱이 가진 마이그레이션 목록에 같은 버전이 둘 이상 있다 (번호 충돌).
    #[error("duplicate migration version v{version} in the app's migration list")]
    DuplicateMigration { version: i64 },

    /// 적용 이력의 최대 버전보다 낮은 미적용 마이그레이션이 있다 (브랜치 번호 충돌 등).
    #[error("migration v{version} is not applied but v{applied_max} is (out-of-order migration)")]
    OutOfOrder { version: i64, applied_max: i64 },

    /// 마이그레이션 SQL 적용이 실패해 롤백됐다. 적용 전 만든 백업이 있으면 경로를 담는다.
    #[error("migration apply failed: {message}")]
    ApplyFailed {
        message: String,
        backup: Option<PathBuf>,
    },

    /// 마이그레이션 적용 후 `PRAGMA foreign_key_check` 가 위반 행을 찾았다.
    /// 커밋 뒤에 검사하므로 복구는 `backup` 으로 한다.
    #[error(
        "foreign key violations after migration ({violations} rows); restore from backup {backup:?}"
    )]
    ForeignKeyViolation {
        violations: usize,
        backup: Option<PathBuf>,
    },

    /// 마이그레이션 목록·이력을 읽는 중의 도구 오류 (refinery).
    #[error("migration tool error: {0}")]
    Migration(String),

    /// 연결 PRAGMA 설정이 기대대로 적용되지 않았다.
    #[error("failed to apply connection pragma: {0}")]
    Pragma(String),

    #[error("backup file already exists: {0}")]
    BackupExists(PathBuf),

    /// 복원할 백업이 없거나, SQLite DB 가 아니거나, 무결성 검사에 실패했다 (대상 DB 는 변하지 않는다).
    #[error("backup {path:?} is unusable: {reason}")]
    BackupInvalid { path: PathBuf, reason: String },
}

impl DbError {
    /// 디스크 공간 부족으로 인한 실패인가 (백업 생성 등에서 사용자 안내를 구분하기 위함).
    pub fn is_disk_full(&self) -> bool {
        match self {
            Self::Sqlite(rusqlite::Error::SqliteFailure(e, _)) => {
                e.code == rusqlite::ErrorCode::DiskFull
            }
            Self::Io(e) => e.kind() == std::io::ErrorKind::StorageFull,
            _ => false,
        }
    }
}
