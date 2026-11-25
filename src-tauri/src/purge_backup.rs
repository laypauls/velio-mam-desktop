//! 연결 데이터를 영구 삭제하는 작업(폴더 제거·유형 정리·기존 보관 데이터 정리·디스크 기록 지우기) 직전의 DB 백업.
//! `mam-pre-remove-<ms>.sqlite` (안전 백업 — 보관 설정과 별개로 최근 3개). 백업이 만들어지지 않거나 검사에 실패하면 `Err` 이고
//! 호출자(`mam_core::ingest::Ingest`)는 아무것도 바꾸지 않는다.

use std::sync::Arc;

use mam_core::SharedClock;
use mam_db::{BackupKind, Clock};

use crate::error::CommandError;
use crate::storage_commands::Storage;

impl Storage {
    /// 일관된 스냅샷(SQLite 백업 API)을 만들고 `quick_check` 로 검사한다. 검사에 실패한 백업은 지운다.
    pub fn backup_before_remove(&self, clock: &dyn Clock) -> Result<String, CommandError> {
        let s = self.db_settings()?;
        let dir = self.backup_dir(&s);
        let protect = crate::update_guard::protected_backups(&self.paths, clock);
        let info = mam_db::create_backup(
            &self.paths.db(),
            &dir,
            BackupKind::PreRemove,
            s.backup.keep as usize,
            &protect,
            clock,
        )?;
        let path = dir.join(&info.file_name);
        if let Err(e) = mam_db::quick_check(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(e.into());
        }
        tracing::info!(backup = %info.file_name, bytes = info.size_bytes, "backup before removal created");
        Ok(info.file_name)
    }
}

/// `Ingest::set_backup_hook` 에 꽂는 콜백. 실패 원인은 로그에만 남는다.
pub fn backup_hook(storage: Arc<Storage>, clock: SharedClock) -> mam_core::ingest::BackupHook {
    Arc::new(move || {
        storage
            .backup_before_remove(&*clock)
            .map(drop)
            .map_err(|e| format!("{:?}", e.code()))
    })
}
