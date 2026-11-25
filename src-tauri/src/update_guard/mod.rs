//! 업데이트 안전장치 — `update-journal.json` 상태 기계와 복구 결정 (P1-7, D3~D5).
//!
//! 흐름: 구 버전이 업데이트 직전 journal(`pending`)을 쓴다 -> 새 버전은 기동할 때마다 실패 횟수를
//! 1 올리고 시작해 창이 뜨면 `healthy` 로 바꾼다 -> 실패 횟수가 한도를 넘으면 `rolling_back` 을 먼저
//! 기록한 뒤 이전 설치기를 실행한다 -> 복구된 구 버전이 첫 기동에서 DB 를 백업으로 되돌리고 `rolled_back`.
//!
//! 결정은 순수 함수 [`decide`] 가 하고, 파일 입출력(`begin`/`mark_*`)은 얇게 감싼다.
//! Tauri·프로세스 실행에 의존하지 않는다 — 시계는 주입한다.

mod journal;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use mam_db::Clock;

pub use journal::{Journal, State};
use journal::{ReadOutcome, delete_if_exists, read_journal, write_journal};

/// 정상 기동에 실패한 횟수가 이 값을 **초과**하면 복구한다 (D4: "실패 횟수 2회 초과").
pub const MAX_FAILED_STARTS: u32 = 2;
/// `rolling_back` 인데 이 시간이 지나도록 복구가 끝나지 않으면 복구를 다시 시도한다.
pub const ROLLBACK_STALE_MS: i64 = 10 * 60 * 1000;

/// MAMData 안의 고정 경로 — 정의는 `mam_core::storage::DataPaths` 한 곳 (P2-1 에서 통합).
pub use mam_core::storage::DataPaths as Paths;

/// 현재 실행 중인 버전이 journal 앞에서 무엇을 해야 하는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// 할 일 없음 (journal 없음·`healthy`·`rolled_back`)
    Proceed,
    /// `pending`, 한도 이내 — 실패 횟수를 올리고 기동을 시도한다
    Attempt,
    /// 새 버전이 한도를 넘어 실패했다 — `rolling_back` 기록 후 이전 설치기를 실행한다
    Rollback {
        installer: PathBuf,
        backup: Option<PathBuf>,
    },
    /// 복구가 이미 진행 중이다 — 아무것도 하지 않고 종료한다
    RollbackInProgress,
    /// 복구된 구 버전의 첫 기동 — DB 를 백업으로 되돌린다
    CompleteRollback { backup: Option<PathBuf> },
    /// 업데이트가 실제로는 적용되지 않았다(설치기 취소 등) — journal 을 정리한다
    Cancelled,
}

/// 순수 결정 함수. `rollback_requested` 는 NSIS 훅이 남긴 표지 파일의 존재 여부.
pub fn decide(
    journal: &Journal,
    current_version: &str,
    rollback_requested: bool,
    now_ms: i64,
) -> Decision {
    let running_old_version = current_version == journal.from_version;
    match journal.state {
        State::Healthy | State::RolledBack => Decision::Proceed,
        State::Pending if running_old_version => {
            if rollback_requested {
                Decision::CompleteRollback {
                    backup: journal.db_backup.clone(),
                }
            } else {
                Decision::Cancelled
            }
        }
        State::Pending if journal.failed_starts <= MAX_FAILED_STARTS => Decision::Attempt,
        State::Pending => rollback(journal),
        State::RollingBack if running_old_version => Decision::CompleteRollback {
            backup: journal.db_backup.clone(),
        },
        State::RollingBack if now_ms - journal.updated_at_ms < ROLLBACK_STALE_MS => {
            Decision::RollbackInProgress
        }
        State::RollingBack => rollback(journal),
    }
}

fn rollback(journal: &Journal) -> Decision {
    Decision::Rollback {
        installer: journal.previous_installer.clone(),
        backup: journal.db_backup.clone(),
    }
}

/// 기동 시 journal 을 읽고 결정해 상태를 기록한다. 호출자가 [`Decision`] 에 따라 동작을 수행한다.
///
/// 깨진 JSON 은 `<journal>.broken-<ms>` 로 보존하고 정상 기동으로 취급한다 (복구하지 않음).
/// `Rollback` 은 `rolling_back` 을 **먼저** 기록하므로 같은 입력으로 다시 호출하면 `RollbackInProgress` 다.
pub fn begin(paths: &Paths, current_version: &str, clock: &dyn Clock) -> std::io::Result<Decision> {
    let now = clock.now_ms();
    let mut journal = match read_journal(&paths.journal(), now)? {
        ReadOutcome::Missing | ReadOutcome::Broken { .. } => return Ok(Decision::Proceed),
        ReadOutcome::Valid(j) => j,
    };
    let decision = decide(
        &journal,
        current_version,
        paths.rollback_flag().exists(),
        now,
    );
    match &decision {
        Decision::Attempt => {
            journal.failed_starts += 1;
            journal.updated_at_ms = now;
            write_journal(&paths.journal(), &journal)?;
        }
        Decision::Rollback { .. } => {
            journal.state = State::RollingBack;
            journal.updated_at_ms = now;
            write_journal(&paths.journal(), &journal)?;
            // 표지를 남기면 이전 설치기의 NSIS 훅이 구 exe 로 사후 검증을 돌려 (새 스키마 DB 라) 실패하고
            // 같은 설치기를 한 번 더 실행한다 (P1-7 시나리오 S3 에서 발견)
            delete_if_exists(&paths.in_progress_marker())?;
        }
        Decision::Cancelled => {
            delete_if_exists(&paths.journal())?;
            delete_if_exists(&paths.in_progress_marker())?;
        }
        Decision::Proceed | Decision::RollbackInProgress | Decision::CompleteRollback { .. } => {}
    }
    Ok(decision)
}

/// 업데이트 직전 호출한다: `pending` journal 과 진행 표지를 쓴다.
pub fn prepare_update(
    paths: &Paths,
    from_version: &str,
    to_version: &str,
    db_backup: Option<PathBuf>,
    clock: &dyn Clock,
) -> std::io::Result<Journal> {
    let now = clock.now_ms();
    let journal = Journal {
        from_version: from_version.to_owned(),
        to_version: to_version.to_owned(),
        previous_installer: paths.current_installer(),
        db_backup,
        state: State::Pending,
        failed_starts: 0,
        created_at_ms: now,
        updated_at_ms: now,
    };
    std::fs::create_dir_all(paths.updates())?;
    delete_if_exists(&paths.rollback_flag())?;
    write_journal(&paths.journal(), &journal)?;
    std::fs::write(paths.in_progress_marker(), b"")?;
    Ok(journal)
}

/// 업데이트를 시작하지 못했을 때(다운로드 실패 등) 준비한 journal·표지를 되돌린다.
pub fn abort_update(paths: &Paths) -> std::io::Result<()> {
    delete_if_exists(&paths.journal())?;
    delete_if_exists(&paths.in_progress_marker())
}

/// 창까지 뜬 정상 기동. `pending` 이면 `healthy` 로 바꾸고 진행 표지를 지운다.
/// 설치기 승격(`pending-setup.exe` -> `current-setup.exe`)은 journal 유무와 무관하게 한다.
pub fn mark_healthy(paths: &Paths, clock: &dyn Clock) -> std::io::Result<()> {
    let now = clock.now_ms();
    if let ReadOutcome::Valid(mut j) = read_journal(&paths.journal(), now)?
        && j.state == State::Pending
    {
        j.state = State::Healthy;
        j.failed_starts = 0;
        j.updated_at_ms = now;
        write_journal(&paths.journal(), &j)?;
    }
    delete_if_exists(&paths.in_progress_marker())?;
    promote_pending_installer(paths)
}

/// 복구 완료(DB 복원까지 끝남) 기록. 표지를 지운다.
pub fn mark_rolled_back(paths: &Paths, clock: &dyn Clock) -> std::io::Result<()> {
    let now = clock.now_ms();
    if let ReadOutcome::Valid(mut j) = read_journal(&paths.journal(), now)? {
        j.state = State::RolledBack;
        j.updated_at_ms = now;
        write_journal(&paths.journal(), &j)?;
    }
    delete_if_exists(&paths.rollback_flag())?;
    delete_if_exists(&paths.in_progress_marker())
}

fn promote_pending_installer(paths: &Paths) -> std::io::Result<()> {
    let pending = paths.pending_installer();
    if pending.is_file() {
        // Windows 의 rename 은 대상이 있으면 덮어쓴다
        std::fs::rename(pending, paths.current_installer())?;
    }
    Ok(())
}

/// 아직 필요한 복구용 DB 백업 — 업데이트가 `pending`/`rolling_back` 인 동안 백업 정리에서 제외한다.
pub fn protected_backups(paths: &Paths, clock: &dyn Clock) -> Vec<PathBuf> {
    match read_journal(&paths.journal(), clock.now_ms()) {
        Ok(ReadOutcome::Valid(j)) if matches!(j.state, State::Pending | State::RollingBack) => {
            j.db_backup.into_iter().collect()
        }
        _ => Vec::new(),
    }
}
