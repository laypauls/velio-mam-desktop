//! 사용자 복원 요청 처리 (`Protection::prepare_restore`) — 재시작은 호출자(command)가 한다.
//!
//! 순서: 인증(PR2: 비밀번호가 있으면 면제 시간과 무관하게 매번) -> 목록에서 백업 찾기 -> 백업 검사(무결성·비어 있지 않음·
//! 스키마 버전 <= 앱) -> `pre_restore` 백업 -> 현재 값(보호·동의·시퀀스)을 담은 표지 기록. 어느 단계든 실패하면 표지를 남기지
//! 않으므로 데이터는 그대로다 (`pre_restore` 백업만 남을 수 있다).

use std::path::PathBuf;

use mam_core::protect::{ProtectError, Zeroizing};
use mam_db::{BackupKind, DbError};

use super::{RestoreMarker, post, write_marker};
use crate::error::{CommandError, ErrorCode};
use crate::protection::Protection;

fn map_db(e: DbError) -> CommandError {
    match e {
        DbError::BackupInvalid { .. } => {
            CommandError::logged(ErrorCode::BackupInvalid, "backup is unusable", &e)
        }
        other => other.into(),
    }
}

impl Protection {
    pub fn prepare_restore(
        &self,
        file_name: &str,
        password: Option<&Zeroizing<String>>,
    ) -> Result<(), CommandError> {
        // 1. 인증 (초기화와 같은 수준: 면제 시간과 무관, 매번)
        if let Some(hash) = self.stored_hash()? {
            let pw = password.ok_or(ProtectError::AuthRequired)?;
            self.guard.check(&hash, pw)?;
        }
        let paths = &self.storage.paths;
        let settings = self.storage.get_settings()?.settings;
        let backup_dir = self.storage.backup_dir(&settings);

        // 2. 목록에 있는 이름만 받는다 (경로 구분자·`..` 가 든 이름은 목록에 없으므로 자연히 거부된다)
        let info = mam_db::list_backups(&backup_dir)
            .map_err(map_db)?
            .into_iter()
            .find(|b| b.file_name == file_name)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "backup not found"))?;
        let backup: PathBuf = backup_dir.join(&info.file_name);

        // 3. 검사: 쓸 수 없는 백업이면 현재 DB 는 건드리지 않는다
        let meta = mam_db::inspect_backup(&backup).map_err(map_db)?;
        if meta.schema_version > mam_db::latest_version() {
            return Err(CommandError::new(
                ErrorCode::BackupTooNew,
                "backup was made by a newer version of the app",
            ));
        }

        // 4. 복원 직전 현재 DB 백업. 고른 백업이 안전 백업 보관 개수 정리에 밀려나지 않게 보호 목록에 넣는다
        let mut protect = crate::update_guard::protected_backups(paths, &*self.clock);
        protect.push(backup.clone());
        mam_db::create_backup(
            &paths.db(),
            &backup_dir,
            BackupKind::PreRestore,
            settings.backup.keep as usize,
            &protect,
            &*self.clock,
        )
        .map_err(map_db)?;

        // 5. 현재 값을 읽어 표지에 싣는다 (표지가 마지막: 있으면 적용이 일어난다)
        let kept = self
            .storage
            .pool
            .read(|c| post::read_kept(c).map_err(DbError::from))?;
        let marker = RestoreMarker {
            requested_at: self.clock.now_ms(),
            backup,
            backup_file: info.file_name,
            backup_created_at: info.created_at,
            kept,
            attempts: 0,
        };
        write_marker(paths, &marker).map_err(|e| ProtectError::Io(e.to_string()))?;
        tracing::info!("restore prepared; restarting to apply");
        Ok(())
    }
}
