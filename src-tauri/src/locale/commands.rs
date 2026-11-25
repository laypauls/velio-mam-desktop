//! `list_locales` / `read_locale` / `open_locale_folder` command. 경로는 프론트에서 받지 않고 고정 위치로 계산한다.

use std::io;
use std::path::Path;
use std::sync::Arc;

use mam_core::reuse::{Launcher, ShellLauncher};
use mam_db::locale_code::is_valid_locale_code;
use serde_json::Value;
use tauri::State;

use super::files::{self, LocaleListing, ReadError};
use super::locales_dir;
use crate::error::{CommandError, ErrorCode};
use crate::storage_commands::Storage;

fn io_error(e: &io::Error) -> CommandError {
    if e.kind() == io::ErrorKind::NotFound {
        CommandError::new(ErrorCode::NotFound, "not found")
    } else {
        CommandError::logged(ErrorCode::Io, "io error", e)
    }
}

pub(super) fn list(dir: &Path) -> Result<LocaleListing, CommandError> {
    let locales = files::list_dir(dir).map_err(|e| io_error(&e))?;
    Ok(LocaleListing {
        dir: dir.display().to_string(),
        locales,
    })
}

pub(super) fn read(dir: &Path, code: &str) -> Result<Value, CommandError> {
    if !is_valid_locale_code(code) {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "invalid locale code",
        ));
    }
    files::read_file(dir, code).map_err(|e| match e {
        ReadError::Invalid(_) => CommandError::new(ErrorCode::InvalidInput, "invalid locale file"),
        ReadError::Io(e) => io_error(&e),
    })
}

/// 폴더가 없으면 만든 뒤 `launcher.open` 으로 연다. 실행 실패는 `open_backup_dir` 와 같게 매핑한다.
pub(super) fn open_folder(dir: &Path, launcher: &dyn Launcher) -> Result<(), CommandError> {
    std::fs::create_dir_all(dir).map_err(|e| io_error(&e))?;
    launcher
        .open(dir)
        .map_err(|e| CommandError::from(mam_core::scan::IngestError::from(e)))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })?
}

#[tauri::command]
pub async fn list_locales(storage: State<'_, Arc<Storage>>) -> Result<LocaleListing, CommandError> {
    let dir = locales_dir(&storage.bootstrap_dir);
    blocking(move || list(&dir)).await
}

#[tauri::command]
pub async fn read_locale(
    storage: State<'_, Arc<Storage>>,
    code: String,
) -> Result<Value, CommandError> {
    let dir = locales_dir(&storage.bootstrap_dir);
    blocking(move || read(&dir, &code)).await
}

#[tauri::command]
pub async fn open_locale_folder(storage: State<'_, Arc<Storage>>) -> Result<(), CommandError> {
    let dir = locales_dir(&storage.bootstrap_dir);
    blocking(move || open_folder(&dir, &ShellLauncher)).await
}
