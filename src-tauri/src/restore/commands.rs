//! 복원 command (`restore_backup`, `take_startup_notice`). 로직은 `Protection::prepare_restore`·`notice` 에 있고
//! 여기서는 인자 변환·`spawn_blocking`·재시작만 한다. 계약 정본: 작업 요청 20261107-123710 의 IPC 표, TS 쪽 `src/lib/ipc.ts`.
//! 비밀번호는 받자마자 `Zeroizing` 으로 감싼다. command 인자는 tracing 에 기록하지 않는다.

use std::sync::Arc;

use mam_core::protect::Zeroizing;
use serde::Deserialize;
use tauri::{AppHandle, State};

use super::notice::{self, StartupNotice};
use crate::error::CommandError;
use crate::protection::Protection;
use crate::storage_commands::Storage;

/// 계약 `RestoreBackupInput`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreInput {
    pub file_name: String,
    pub password: Option<String>,
}

/// 인증 -> 검증 -> `pre_restore` 백업 -> 표지 -> 서비스 정지 -> 재시작. 성공하면 앱이 다시 시작되므로 응답이 오지 않는다.
/// 오류 코드: `auth_required`, `password_incorrect`, `too_many_attempts`, `backup_invalid`, `backup_too_new`, `not_found`.
#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    svc: State<'_, Arc<Protection>>,
    input: RestoreInput,
) -> Result<(), CommandError> {
    let worker = Arc::clone(&svc);
    tauri::async_runtime::spawn_blocking(move || {
        let password = input.password.map(Zeroizing::new);
        worker.prepare_restore(&input.file_name, password.as_ref())?;
        // 표지가 쓰였다 — 작업·감시 스레드와 DB 쓰기를 멈추고 다시 시작한다 (교체는 새 프로세스가 DB 를 열기 전에 한다)
        worker.ingest.shutdown();
        worker.storage.pool.shutdown();
        Ok::<(), CommandError>(())
    })
    .await
    .map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })??;
    app.restart()
}

/// 기동 알림을 한 번만 돌려준다 (읽으면 지워진다). 없으면 `null`.
#[tauri::command]
pub async fn take_startup_notice(
    storage: State<'_, Arc<Storage>>,
) -> Result<Option<StartupNotice>, CommandError> {
    let storage = Arc::clone(&storage);
    tauri::async_runtime::spawn_blocking(move || notice::take(&storage.paths))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })
}
