//! 관리 데이터 초기화 (P2-9 D5) — 앱 쪽 순서:
//!
//! 요청 시 (`Protection::prepare_reset`): 인증 -> `pre_reset` 백업(P2-1) -> `plan_reset` -> 보호 경로 사이드카 ->
//! `reset-pending` 표지 -> (command 가) 서비스 정지 후 재시작.
//! 시작 시 (`apply_pending`): 단일 인스턴스 잠금 뒤, **DB 를 열기 전에** 표지를 적용한다 (`startup::boot`) -> 이어서 마이그레이션.
//!
//! 보호 경로(관리 폴더 루트·사용자 지정 백업 폴더)는 DB 에 있어 시작 시점에는 읽을 수 없으므로(곧 지울 DB) 요청 시점에
//! `reset-protected.json` 사이드카로 남긴다. 원본·백업 폴더는 삭제 대상이 아니다.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mam_core::protect::{
    BackupOutcome, ProtectError, Zeroizing, apply_pending_reset, clear_reset_marker, marker_path,
    plan_reset, write_reset_marker,
};
use mam_core::storage::{DataPaths, InstanceLock, InstanceMutex, StorageError};
use mam_db::BackupKind;

use crate::error::CommandError;
use crate::protection::Protection;

const SIDECAR_FILE: &str = "reset-protected.json";
/// 재시작한 새 프로세스가 이전 프로세스의 종료(잠금 해제)를 기다리는 최대 시간
const LOCK_WAIT: Duration = Duration::from_secs(10);

fn sidecar_path(paths: &DataPaths) -> PathBuf {
    paths.root().join(SIDECAR_FILE)
}

fn read_sidecar(paths: &DataPaths) -> Vec<PathBuf> {
    let mut protected = vec![paths.backups()];
    match std::fs::read(sidecar_path(paths)) {
        Ok(bytes) => match serde_json::from_slice::<Vec<PathBuf>>(&bytes) {
            Ok(list) => protected.extend(list),
            Err(_) => tracing::warn!("reset-protected.json is corrupt; using defaults"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(%e, "cannot read reset-protected.json"),
    }
    protected
}

/// 초기화 표지가 남아 있는가 (재시작 중이거나 이전 적용이 중단됐다)
pub fn is_pending(paths: &DataPaths) -> bool {
    marker_path(paths).is_file()
}

/// 단일 인스턴스 잠금. 초기화 재시작이면 이전 프로세스가 끝날 때까지 잠시 기다린다 —
/// 그렇지 않으면 새 프로세스가 "다른 인스턴스 실행 중" 으로 판단해 앱이 사라진다.
pub fn acquire_lock(paths: &DataPaths) -> Result<InstanceLock, StorageError> {
    retry_while_pending(paths, LOCK_WAIT, || {
        InstanceLock::acquire(&paths.lock_file())
    })
}

/// 사용자 세션 단위 단일 인스턴스 뮤텍스. `acquire_lock` 과 같은 초기화 재시작 대기를 적용한다.
pub fn acquire_instance_mutex(
    paths: &DataPaths,
    identifier: &str,
) -> Result<InstanceMutex, StorageError> {
    acquire_instance_mutex_within(paths, identifier, LOCK_WAIT)
}

pub(crate) fn acquire_instance_mutex_within(
    paths: &DataPaths,
    identifier: &str,
    wait: Duration,
) -> Result<InstanceMutex, StorageError> {
    retry_while_pending(paths, wait, || InstanceMutex::acquire(identifier))
}

fn retry_while_pending<T>(
    paths: &DataPaths,
    wait: Duration,
    mut attempt: impl FnMut() -> Result<T, StorageError>,
) -> Result<T, StorageError> {
    let started = Instant::now();
    loop {
        match attempt() {
            Err(StorageError::AlreadyRunning)
                if (is_pending(paths) || crate::restore::is_pending(paths))
                    && started.elapsed() < wait =>
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
#[path = "reset_instance_tests.rs"]
mod instance_tests;

#[derive(Debug, PartialEq, Eq)]
pub enum StartupReset {
    /// 표지 없음
    None,
    /// 적용했다 (백업 파일 경로)
    Applied(PathBuf),
    /// 계획 검증을 통과하지 못해 아무것도 지우지 않고 표지를 버렸다
    Cancelled(String),
}

/// 시작 시 호출: DB 를 열기 전에. 입출력 오류(`Err`)는 표지를 남겨 다음 시작에 다시 시도한다(삭제는 멱등).
pub fn apply_pending(paths: &DataPaths) -> Result<StartupReset, ProtectError> {
    if !is_pending(paths) {
        return Ok(StartupReset::None);
    }
    let protected = read_sidecar(paths);
    match apply_pending_reset(paths, &protected) {
        Ok(Some(report)) => {
            remove_sidecar(paths);
            Ok(StartupReset::Applied(report.backup))
        }
        Ok(None) => Ok(StartupReset::None),
        Err(e @ ProtectError::Io(_)) => Err(e),
        Err(other) => {
            // 표지가 손상됐거나 백업이 없어졌다 — 지우지 않고 표지만 버린다 (앱이 영영 못 뜨는 일을 막는다)
            clear_reset_marker(paths)?;
            remove_sidecar(paths);
            Ok(StartupReset::Cancelled(other.to_string()))
        }
    }
}

fn remove_sidecar(paths: &DataPaths) {
    let _ = std::fs::remove_file(sidecar_path(paths));
}

impl Protection {
    /// 인증 -> 백업 -> 계획 -> 표지까지. 재시작은 호출자(command)가 한다.
    /// 어느 단계든 실패하면 표지를 남기지 않으므로 데이터는 그대로다.
    pub fn prepare_reset(
        &self,
        password: Option<&Zeroizing<String>>,
        forgot_password: bool,
    ) -> Result<(), CommandError> {
        // 1. 인증: 분실 경로(PL3)는 인증 없이, 그 외에는 비밀번호 확인(면제 시간과 무관, 매번)
        if !forgot_password && let Some(hash) = self.stored_hash()? {
            let pw = password.ok_or(ProtectError::AuthRequired)?;
            self.guard.check(&hash, pw)?;
        }
        let paths = &self.storage.paths;
        let settings = self.storage.get_settings()?.settings;
        let backup_dir = self.storage.backup_dir(&settings);

        // 2. 초기화 직전 백업 — 실패하면 중단 (분실 경로도 백업은 시도한다)
        let protect_backups = crate::update_guard::protected_backups(paths, &*self.clock);
        let outcome = match mam_db::create_backup(
            &paths.db(),
            &backup_dir,
            BackupKind::PreReset,
            settings.backup.keep as usize,
            &protect_backups,
            &*self.clock,
        ) {
            Ok(info) => BackupOutcome::Created(backup_dir.join(info.file_name)),
            Err(e) => BackupOutcome::Failed(e.to_string()),
        };

        // 3. 계획: 관리 폴더 루트와 사용자 지정 백업 폴더는 지우지 않는다
        let mut protected: Vec<PathBuf> = self
            .ingest
            .list_folders()
            .map_err(CommandError::from)?
            .into_iter()
            .map(|f| PathBuf::from(f.path))
            .collect();
        protected.push(backup_dir);
        let plan = plan_reset(paths, &outcome, &protected)?;

        // 4. 사이드카 -> 표지 (표지가 마지막: 있으면 적용이 일어난다)
        write_sidecar(paths, &protected)?;
        if let Err(e) = write_reset_marker(paths, &*self.clock, &plan) {
            remove_sidecar(paths);
            return Err(e.into());
        }
        tracing::info!("reset prepared; restarting to apply");
        Ok(())
    }
}

fn write_sidecar(paths: &DataPaths, protected: &[PathBuf]) -> Result<(), ProtectError> {
    let json = serde_json::to_vec(protected).map_err(|e| ProtectError::Internal(e.to_string()))?;
    let dest = sidecar_path(paths);
    let tmp: &Path = &dest.with_extension("json.tmp");
    std::fs::write(tmp, json)
        .and_then(|()| std::fs::rename(tmp, &dest))
        .map_err(|e| ProtectError::Io(format!("{}: {e}", dest.display())))
}
