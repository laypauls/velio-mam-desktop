//! 기동 순서 — 업데이트 안전장치(journal) -> DB 열기·마이그레이션 -> (호출자가) 창 생성 (P1-7).
//!
//! `--verify-update` 모드는 같은 DB 열기·마이그레이션만 하고 UI 없이 종료 코드로 결과를 알린다
//! (NSIS 설치 직후 훅이 호출). MAMData 위치 결정·디렉터리 생성·단일 인스턴스 잠금은 호출자(`lib.rs`)가 이 함수 전에 끝낸다 —
//! 이 모듈은 이미 정해진 `Paths` 안에서 journal 처리와 DB 열기만 한다.

mod launcher;
#[cfg(test)]
mod tests;

use std::io::Write;
use std::path::PathBuf;

use mam_db::{Clock, DbError};

pub use launcher::{Launcher, ProcessLauncher};

use crate::update_guard::{self, Decision, Paths};

pub const VERIFY_FLAG: &str = "--verify-update";

/// 기동 결과: 앱을 계속 실행하거나, 이 종료 코드로 즉시 끝낸다.
#[derive(Debug, PartialEq, Eq)]
pub enum Boot {
    Run,
    Exit(i32),
}

pub fn boot(
    args: &[String],
    version: &str,
    paths: &Paths,
    clock: &dyn Clock,
    launcher: &dyn Launcher,
) -> Boot {
    if args.iter().any(|a| a == VERIFY_FLAG) {
        return match verify(paths, clock) {
            Ok(()) => Boot::Exit(0),
            Err(e) => {
                log(paths, clock, &format!("verify-update failed: {e}"));
                Boot::Exit(1)
            }
        };
    }

    if let Err(e) = std::fs::create_dir_all(paths.root()) {
        eprintln!("cannot create data dir {:?}: {e}", paths.root());
        return Boot::Exit(1);
    }
    // journal 입출력 오류는 기동을 막지 않는다 — 복구 장치 때문에 정상 앱이 안 뜨면 안 된다
    match update_guard::begin(paths, version, clock) {
        Err(e) => log(paths, clock, &format!("update journal error: {e}")),
        Ok(Decision::Rollback { installer, .. }) => {
            log(paths, clock, "new version failed to start; rolling back");
            return match launcher.launch(&installer) {
                Ok(()) => Boot::Exit(0),
                Err(e) => {
                    log(paths, clock, &format!("rollback installer failed: {e}"));
                    Boot::Exit(1)
                }
            };
        }
        Ok(Decision::RollbackInProgress) => return Boot::Exit(0),
        Ok(Decision::CompleteRollback { backup }) => complete_rollback(paths, clock, backup),
        Ok(Decision::Attempt | Decision::Proceed | Decision::Cancelled) => {}
    }

    // 관리 데이터 초기화 표지(P2-9)는 journal 처리(DB 복원) **뒤**, DB 를 열기 **전**에 적용한다.
    // 입출력 오류면 표지를 남기고 종료한다 (반쯤 지워진 DB 를 열지 않는다 — 다음 시작에 이어서 지운다).
    match crate::reset::apply_pending(paths) {
        Ok(crate::reset::StartupReset::None) => {}
        Ok(crate::reset::StartupReset::Applied(backup)) => log(
            paths,
            clock,
            &format!("managed data reset applied (backup: {})", backup.display()),
        ),
        Ok(crate::reset::StartupReset::Cancelled(why)) => {
            log(paths, clock, &format!("pending reset cancelled: {why}"));
        }
        Err(e) => {
            log(paths, clock, &format!("pending reset failed: {e}"));
            return Boot::Exit(1);
        }
    }

    // DB 백업 복원 표지(요청 20261107-123710)는 초기화 표지 **뒤**, DB 를 열기 **전**에 적용한다 (같은 이유).
    // 입출력 오류면 표지를 남기고 종료한다 (한도 횟수까지 다음 기동에서 같은 순서로 다시 한다 — 멱등).
    match crate::restore::apply_pending(paths) {
        Ok(crate::restore::StartupRestore::None) => {}
        Ok(crate::restore::StartupRestore::Applied(file)) => {
            log(
                paths,
                clock,
                &format!("backup restore applied (backup: {file})"),
            );
        }
        Ok(crate::restore::StartupRestore::Discarded(why)) => {
            log(paths, clock, &format!("pending restore discarded: {why}"));
        }
        Err(e) => {
            log(paths, clock, &format!("pending restore failed: {e}"));
            return Boot::Exit(1);
        }
    }

    let started = open_or_recover(paths, clock).and_then(|()| inject_startup_failure());
    match started {
        Ok(()) => Boot::Run,
        Err(e) => {
            log(paths, clock, &format!("startup failed: {e}"));
            Boot::Exit(1)
        }
    }
}

fn complete_rollback(paths: &Paths, clock: &dyn Clock, backup: Option<PathBuf>) {
    if let Some(backup) = backup {
        // 연결을 열기 전에 복원한다 — 새 스키마 DB 를 구 러너가 열면 SchemaTooNew 로 막힌다
        if let Err(e) = mam_db::restore_backup(&backup, &paths.db()) {
            log(
                paths,
                clock,
                &format!("DB restore from {backup:?} failed: {e}"),
            );
            return; // rolled_back 으로 기록하지 않아 다음 기동에서 다시 시도한다
        }
    }
    if let Err(e) = update_guard::mark_rolled_back(paths, clock) {
        log(paths, clock, &format!("cannot record rollback: {e}"));
    }
}

fn verify(paths: &Paths, clock: &dyn Clock) -> Result<(), String> {
    #[cfg(feature = "inject-fail-verify")]
    return Err("injected verify failure (feature inject-fail-verify)".into());
    #[cfg(not(feature = "inject-fail-verify"))]
    open_and_migrate(paths, clock).map_err(|e| e.to_string())
}

fn inject_startup_failure() -> Result<(), String> {
    #[cfg(feature = "inject-fail-startup")]
    return Err("injected startup failure (feature inject-fail-startup)".into());
    #[cfg(not(feature = "inject-fail-startup"))]
    Ok(())
}

/// 열기·마이그레이션. `SQLITE_CORRUPT`/`SQLITE_NOTADB` 로 실패했을 때만 자동 복구(PR3) 후 한 번 다시 연다 —
/// 잠금·I/O·권한 오류는 손상이 아닐 수 있어 DB 를 옮기지 않고 그대로 실패한다 (H5).
fn open_or_recover(paths: &Paths, clock: &dyn Clock) -> Result<(), String> {
    let err = match open_and_migrate(paths, clock) {
        Ok(()) => return Ok(()),
        Err(e) if crate::restore::recover::is_corruption(&e) => e,
        Err(e) => return Err(e.to_string()),
    };
    log(
        paths,
        clock,
        &format!("database is corrupt; recovering: {err}"),
    );
    let recovered = crate::restore::recover::recover(paths, clock)?;
    log(
        paths,
        clock,
        &format!(
            "database recovered (corrupt file kept at {}; backup: {:?}; consent reset: {})",
            recovered.corrupt_file.display(),
            recovered.backup.as_ref().map(|b| &b.0),
            recovered.consent_reset
        ),
    );
    open_and_migrate(paths, clock).map_err(|e| e.to_string())
}

fn open_and_migrate(paths: &Paths, clock: &dyn Clock) -> Result<(), DbError> {
    std::fs::create_dir_all(paths.database_dir())?;
    let mut conn = mam_db::open(&paths.db())?;
    mam_db::migrate(&mut conn, &paths.backups(), clock)?;
    Ok(())
}

/// `<MAMData>/logs/startup.log` 에 한 줄 덧붙인다. 실패해도 무시한다(로그가 기동을 막지 않는다).
fn log(paths: &Paths, clock: &dyn Clock, message: &str) {
    eprintln!("{message}");
    tracing::warn!("{message}");
    let dir = paths.root().join("logs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("startup.log"))
    {
        let _ = writeln!(f, "{} {message}", clock.now_ms());
    }
}
