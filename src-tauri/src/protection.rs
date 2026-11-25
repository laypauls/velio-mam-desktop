//! 보호 서비스 (P2-9): 비밀번호 설정·변경·검증, 원본 삭제(휴지통)·이동. 로직은 `mam_core::protect` 에 있고
//! 여기서는 PHC 해시 저장(설정 계층), 면제 시간 설정 읽기, 전용 DB 연결, 오류 변환을 맡는다.
//!
//! 휴지통 호출은 확인창을 띄우며 오래 막힐 수 있어 DB 쓰기 스레드(풀)에서 실행하지 않는다 — 호출마다 별도 연결을 열고,
//! command 는 `spawn_blocking` 에서 부른다. 비밀번호 평문은 `Zeroizing` 으로만 들고 다니며 로그에 남기지 않는다.
//! 초기화(`prepare_reset`)는 `reset.rs`, command 는 `protect_commands.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use mam_core::ingest::Ingest;
use mam_core::protect::{
    self, MoveEnv, OriginalsEnv, OriginalsResult, ProtectError, ProtectionGuard, ProtectionStatus,
    Trash, Zeroizing,
};
use mam_core::{
    Blake3Hasher, ContentHasher, FileIdentityProvider, SharedClock, VolumeProvider,
    WindowsFileIdentity, WindowsVolumes,
};
use mam_db::{Priority, settings};

use crate::error::{CommandError, ErrorCode};
use crate::storage_commands::Storage;

/// 한 번에 다루는 자산 수 상한 (사용자 데이터 command 와 같다)
pub const MAX_ASSETS_PER_CALL: usize = 10_000;

impl From<ProtectError> for CommandError {
    fn from(e: ProtectError) -> Self {
        // 메시지는 고정 문구 — 경로·해시·평문은 싣지 않고, 원인은 (비밀번호 없이) 로그에만 남긴다
        match e {
            ProtectError::InvalidInput(_) => Self::new(ErrorCode::InvalidInput, "invalid input"),
            ProtectError::PasswordNotSet => {
                Self::new(ErrorCode::PasswordNotSet, "password is not set")
            }
            ProtectError::PasswordAlreadySet => {
                Self::new(ErrorCode::PasswordAlreadySet, "password is already set")
            }
            ProtectError::PasswordIncorrect => {
                Self::new(ErrorCode::PasswordIncorrect, "password is incorrect")
            }
            ProtectError::AuthRequired => {
                Self::new(ErrorCode::AuthRequired, "authentication required")
            }
            ProtectError::TooManyAttempts { .. } => {
                Self::new(ErrorCode::TooManyAttempts, "too many failed attempts")
            }
            ProtectError::NotFound(_) => Self::new(ErrorCode::NotFound, "not found"),
            ProtectError::Offline(_) => Self::new(ErrorCode::Offline, "volume is offline"),
            ProtectError::Io(_) | ProtectError::BackupFailed(_) => {
                Self::logged(ErrorCode::Io, "file operation failed", &e)
            }
            ProtectError::Internal(_) => {
                Self::logged(ErrorCode::Internal, "protection operation failed", &e)
            }
        }
    }
}

/// 원본에 닿는 외부 의존성 (시험에서는 가짜 휴지통을 넣는다)
pub struct OriginalsDeps {
    pub trash: Arc<dyn Trash + Send + Sync>,
    pub volumes: Arc<dyn VolumeProvider + Send + Sync>,
    pub identity: Arc<dyn FileIdentityProvider + Send + Sync>,
    pub hasher: Arc<dyn ContentHasher + Send + Sync>,
}

impl OriginalsDeps {
    pub fn system() -> Self {
        Self {
            trash: Arc::new(protect::RecycleBin),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        }
    }
}

pub struct Protection {
    pub(crate) storage: Arc<Storage>,
    pub(crate) ingest: Arc<Ingest>,
    pub(crate) guard: ProtectionGuard,
    pub(crate) clock: SharedClock,
    deps: OriginalsDeps,
}

impl Protection {
    pub fn new(
        storage: Arc<Storage>,
        ingest: Arc<Ingest>,
        clock: SharedClock,
        deps: OriginalsDeps,
    ) -> Self {
        Self {
            guard: ProtectionGuard::new(Arc::clone(&clock)),
            storage,
            ingest,
            clock,
            deps,
        }
    }

    pub(crate) fn stored_hash(&self) -> Result<Option<String>, CommandError> {
        Ok(self.storage.pool.read(settings::load_password_hash)?)
    }

    pub fn status(&self) -> Result<ProtectionStatus, CommandError> {
        Ok(self.guard.status(self.stored_hash()?.is_some()))
    }

    /// 해시를 compare-and-set 으로 저장한다. 그 사이 다른 호출이 바꿨으면 `conflict` 오류.
    fn store_hash(
        &self,
        expected: Option<String>,
        new: String,
        conflict: ProtectError,
    ) -> Result<(), CommandError> {
        let swapped = self.storage.pool.write(Priority::High, move |c| {
            settings::swap_password_hash(c, expected.as_deref(), &new)
        })?;
        if swapped {
            Ok(())
        } else {
            Err(conflict.into())
        }
    }

    /// 최초 설정만. 인증 면제는 시작하지 않는다.
    pub fn set_password(
        &self,
        password: &Zeroizing<String>,
    ) -> Result<ProtectionStatus, CommandError> {
        let stored = self.stored_hash()?;
        let phc = self.guard.set_password(stored.as_deref(), password)?;
        self.store_hash(None, phc, ProtectError::PasswordAlreadySet)?;
        self.status()
    }

    pub fn change_password(
        &self,
        current: &Zeroizing<String>,
        next: &Zeroizing<String>,
    ) -> Result<ProtectionStatus, CommandError> {
        let stored = self.stored_hash()?;
        let phc = self
            .guard
            .change_password(stored.as_deref(), current, next)?;
        self.store_hash(
            stored,
            phc,
            ProtectError::Internal("password changed concurrently".into()),
        )?;
        self.status()
    }

    /// 성공하면 `authExemptMinutes` 동안 보호 명령을 허용한다.
    pub fn verify_password(
        &self,
        password: &Zeroizing<String>,
    ) -> Result<ProtectionStatus, CommandError> {
        let stored = self.stored_hash()?;
        let minutes = self.storage.get_settings()?.settings.auth_exempt_minutes;
        Ok(self
            .guard
            .verify_password(stored.as_deref(), password, minutes)?)
    }

    fn check_count(ids: &[i64]) -> Result<(), CommandError> {
        if ids.len() > MAX_ASSETS_PER_CALL {
            return Err(ProtectError::InvalidInput("too many assets".into()).into());
        }
        Ok(())
    }

    fn originals_env(&self, password_set: bool) -> OriginalsEnv<'_> {
        OriginalsEnv {
            guard: &self.guard,
            password_set,
            clock: &*self.clock,
            trash: &*self.deps.trash,
        }
    }

    /// 휴지통으로 이동. 인증 면제 밖이면 `auth_required` 이고 아무것도 건드리지 않는다.
    pub fn delete_originals(&self, ids: &[i64]) -> Result<OriginalsResult, CommandError> {
        Self::check_count(ids)?;
        let password_set = self.stored_hash()?.is_some();
        let mut conn = mam_db::open(&self.storage.paths.db())?;
        let result = protect::delete_originals(&mut conn, &self.originals_env(password_set), ids)?;
        tracing::info!(
            succeeded = result.succeeded,
            failed = result.failed.len(),
            "originals moved to recycle bin"
        );
        Ok(result)
    }

    /// 대상 폴더로 이동. 관리 폴더 안으로 옮기는 동안 그 폴더의 감시 이벤트를 억제한다.
    pub fn move_originals(
        &self,
        ids: &[i64],
        dest_dir: &str,
    ) -> Result<OriginalsResult, CommandError> {
        Self::check_count(ids)?;
        if dest_dir.is_empty() || dest_dir.contains('\0') {
            return Err(ProtectError::InvalidInput("destination".into()).into());
        }
        let dest = PathBuf::from(dest_dir);
        let password_set = self.stored_hash()?.is_some();
        let mut conn = mam_db::open(&self.storage.paths.db())?;
        // MAMData·백업 폴더 안으로는 옮기지 않는다 — 초기화가 지우거나 스캔에서 빠지는 위치 (보안 리뷰 #2)
        let custom_backup = self.storage.backup_dir(&self.storage.db_settings()?);
        let excluded = [
            self.storage.paths.root().to_path_buf(),
            self.storage.paths.backups(),
            custom_backup,
        ];
        let env = MoveEnv {
            base: self.originals_env(password_set),
            volumes: &*self.deps.volumes,
            identity: &*self.deps.identity,
            hasher: &*self.deps.hasher,
            excluded_dirs: &excluded,
        };
        let _suppress = self.ingest.suppress_watch(&dest);
        let report = protect::move_originals(&mut conn, &env, ids, &dest)?;
        tracing::info!(
            succeeded = report.result.succeeded,
            failed = report.result.failed.len(),
            detached = report.detached_asset_ids.len(),
            "originals moved"
        );
        Ok(report.result)
    }
}

#[cfg(test)]
#[path = "protection_tests.rs"]
mod tests;
