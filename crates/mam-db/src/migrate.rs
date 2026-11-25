//! 마이그레이션 러너 래퍼 (refinery 위에 얹는다).
//!
//! 순서: 사전 검증(새 스키마·체크섬·번호 누락) -> 필요 시 백업 -> 트랜잭션 적용.
//! 사전 검증을 백업보다 먼저 하므로 오류로 끝나는 실행은 DB 도, 백업 디렉터리도 건드리지 않는다.
//!
//! 적용은 `set_grouped(true)` — 대기 중인 모든 마이그레이션과 이력 기록이 **하나의 트랜잭션**이다.
//! (grouped=false 는 마이그레이션 SQL 과 이력 INSERT 가 서로 다른 트랜잭션이라, 그 사이에 죽으면
//! 스키마는 바뀌었는데 이력이 없는 상태가 된다.)

use std::path::{Path, PathBuf};

use refinery::{Migration, Runner};
use rusqlite::{Connection, MAIN_DB};

use crate::error::DbError;

mod embedded {
    refinery::embed_migrations!("migrations");
}

const HISTORY_TABLE: &str = "refinery_schema_history";

/// 시각 주입점 — 백업 파일명에 쓰는 현재 시각(UTC Unix ms)을 테스트에서 고정하기 위함.
pub trait Clock {
    fn now_ms(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// 이번 실행에서 새로 적용한 마이그레이션 버전 (오름차순)
    pub applied: Vec<i64>,
    /// 적용 전 만든 백업 파일 (기존 스키마가 있는 DB 에 대기 마이그레이션이 있을 때만)
    pub backup: Option<PathBuf>,
}

/// 앱에 포함된 마이그레이션 중 가장 높은 버전.
pub fn latest_version() -> i64 {
    embedded::migrations::runner()
        .get_migrations()
        .iter()
        .map(|m| i64::from(m.version()))
        .max()
        .unwrap_or(0)
}

/// 앱에 포함된 마이그레이션을 적용한다.
pub fn migrate(
    conn: &mut Connection,
    backup_dir: &Path,
    clock: &dyn Clock,
) -> Result<MigrateReport, DbError> {
    let known = embedded::migrations::runner().get_migrations().clone();
    migrate_with(conn, &known, backup_dir, clock)
}

/// 주어진 마이그레이션 목록을 적용한다 (테스트에서 실패·변조 마이그레이션을 주입하기 위한 진입점).
pub fn migrate_with(
    conn: &mut Connection,
    known: &[Migration],
    backup_dir: &Path,
    clock: &dyn Clock,
) -> Result<MigrateReport, DbError> {
    let known = &normalize_line_endings(known)?;
    ensure_unique_versions(known)?;
    let runner = Runner::new(known)
        .set_grouped(true)
        .set_abort_divergent(true)
        .set_abort_missing(true);

    let applied = if has_history_table(conn)? {
        runner
            .get_applied_migrations(conn)
            .map_err(|e| DbError::Migration(e.to_string()))?
    } else {
        Vec::new()
    };

    let max_known = known
        .iter()
        .map(|m| i64::from(m.version()))
        .max()
        .unwrap_or(0);
    let max_applied = applied.iter().map(|m| i64::from(m.version())).max();

    // 새 스키마 감지를 먼저: 롤백 시나리오에서는 "앱이 더 낡았다"가 체크섬 오류보다 정확한 안내다
    if let Some(db) = max_applied
        && db > max_known
    {
        return Err(DbError::SchemaTooNew { db, app: max_known });
    }
    for a in &applied {
        let version = i64::from(a.version());
        match known.iter().find(|m| m.version() == a.version()) {
            None => return Err(DbError::UnknownMigration { version }),
            Some(k) if k != a => {
                return Err(DbError::ChecksumMismatch {
                    version,
                    name: a.name().to_owned(),
                });
            }
            Some(_) => {}
        }
    }

    let mut pending: Vec<i64> = known
        .iter()
        .filter(|m| !applied.iter().any(|a| a.version() == m.version()))
        .map(|m| i64::from(m.version()))
        .collect();
    pending.sort_unstable();

    if let (Some(&first), Some(applied_max)) = (pending.first(), max_applied)
        && first < applied_max
    {
        return Err(DbError::OutOfOrder {
            version: first,
            applied_max,
        });
    }

    if pending.is_empty() {
        return Ok(MigrateReport::default());
    }

    // 기존 스키마가 있는 DB(적용 이력 있음)만 백업한다. 새 DB 는 잃을 데이터가 없다.
    let backup = match max_applied {
        Some(version) => Some(backup_before_migrate(conn, backup_dir, clock, version)?),
        None => None,
    };

    apply_with_fk_off(conn, &runner, backup.as_deref())?;

    Ok(MigrateReport {
        applied: pending,
        backup,
    })
}

/// 적용 구간에서만 `foreign_keys = OFF` 로 둔다.
///
/// 테이블 재생성(12단계 방식)의 `DROP TABLE` 은 FK 가 켜져 있으면 암묵적 DELETE 를 수행해
/// 자식 행을 CASCADE 로 지우거나 RESTRICT 로 실패한다. `PRAGMA foreign_keys` 는 트랜잭션 안에서는
/// 무시되고 refinery 는 전체를 한 트랜잭션으로 실행하므로, 트랜잭션 **밖인 여기서** 끈다.
/// 따라서 마이그레이션 SQL 안에는 PRAGMA 를 쓰지 않는다.
///
/// 약점: `foreign_key_check` 는 refinery 가 커밋한 **뒤**에 돌 수밖에 없다 (커밋 전 훅 없음).
/// 위반이 발견돼도 이미 커밋된 상태이므로 복구는 적용 전 백업으로 한다.
/// (새 DB 는 잃을 데이터가 없다.) 결과와 무관하게 `foreign_keys` 는 항상 다시 ON 으로 복원한다.
fn apply_with_fk_off(
    conn: &mut Connection,
    runner: &Runner,
    backup: Option<&Path>,
) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", false)?;
    let run = runner.run(conn);
    let violations = match &run {
        Ok(_) => count_foreign_key_violations(conn),
        Err(_) => Ok(0),
    };
    let restore = conn.pragma_update(None, "foreign_keys", true);

    if let Err(e) = run {
        return Err(DbError::ApplyFailed {
            message: e.to_string(),
            backup: backup.map(Path::to_path_buf),
        });
    }
    let violations = violations?;
    restore?;
    if violations > 0 {
        return Err(DbError::ForeignKeyViolation {
            violations,
            backup: backup.map(Path::to_path_buf),
        });
    }
    Ok(())
}

fn count_foreign_key_violations(conn: &Connection) -> Result<usize, DbError> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    let mut n = 0;
    while rows.next()?.is_some() {
        n += 1;
    }
    Ok(n)
}

fn ensure_unique_versions(known: &[Migration]) -> Result<(), DbError> {
    let mut seen = std::collections::HashSet::new();
    for m in known {
        if !seen.insert(m.version()) {
            return Err(DbError::DuplicateMigration {
                version: i64::from(m.version()),
            });
        }
    }
    Ok(())
}

/// refinery 체크섬은 SQL 텍스트 전체로 계산되므로, CRLF 체크아웃이 LF 로 적용된 이력과
/// 체크섬이 달라지는 것을 막기 위해 줄바꿈을 LF 로 통일한다.
fn normalize_line_endings(known: &[Migration]) -> Result<Vec<Migration>, DbError> {
    known
        .iter()
        .map(|m| {
            let sql = m.sql().unwrap_or_default().replace("\r\n", "\n");
            Migration::unapplied(&m.to_string(), &sql)
                .map_err(|e| DbError::Migration(e.to_string()))
        })
        .collect()
}

fn has_history_table(conn: &Connection) -> Result<bool, DbError> {
    let found = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [HISTORY_TABLE],
        |r| r.get::<_, bool>(0),
    )?;
    Ok(found)
}

/// SQLite 백업 API 로 일관된 스냅샷을 만든다 (실행 중 DB 파일 단순 복사 금지 — WAL 미반영).
fn backup_before_migrate(
    conn: &Connection,
    backup_dir: &Path,
    clock: &dyn Clock,
    from_version: i64,
) -> Result<PathBuf, DbError> {
    std::fs::create_dir_all(backup_dir)?;
    let path = backup_dir.join(format!(
        "mam-pre-migrate-v{from_version}-{}.sqlite",
        clock.now_ms()
    ));
    if path.exists() {
        return Err(DbError::BackupExists(path));
    }
    if let Err(e) = conn.backup(MAIN_DB, &path, None) {
        let _ = std::fs::remove_file(&path);
        return Err(e.into());
    }
    Ok(path)
}
