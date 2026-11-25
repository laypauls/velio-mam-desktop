//! 보호 command (P2-9): 비밀번호, 원본 삭제·이동, 관리 데이터 초기화. 로직은 `Protection` 에 있고 여기서는
//! 인자 변환·`spawn_blocking`·재시작만 한다. 계약 정본: UI 계약 `ipc-contract.ts` (P2-9), TS 쪽 `src/lib/ipc.ts`.
//!
//! 비밀번호는 받자마자 `Zeroizing` 으로 감싼다. command 인자는 tracing 에 기록하지 않는다.

use std::sync::Arc;

use mam_core::protect::{OriginalsResult, ProtectionStatus, Zeroizing};
use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::protection::Protection;

type Shared<'a> = State<'a, Arc<Protection>>;

async fn blocking<T, F>(svc: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Protection) -> Result<T, CommandError> + Send + 'static,
{
    let svc = Arc::clone(svc);
    tauri::async_runtime::spawn_blocking(move || f(&svc))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
}

/// 계약 `ResetInput`
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetInput {
    pub password: Option<String>,
    pub forgot_password: bool,
}

#[tauri::command]
pub async fn get_protection_status(svc: Shared<'_>) -> Result<ProtectionStatus, CommandError> {
    blocking(&svc, Protection::status).await
}

#[tauri::command]
pub async fn set_password(
    svc: Shared<'_>,
    password: String,
) -> Result<ProtectionStatus, CommandError> {
    let password = Zeroizing::new(password);
    blocking(&svc, move |p| p.set_password(&password)).await
}

#[tauri::command]
pub async fn change_password(
    svc: Shared<'_>,
    current: String,
    next: String,
) -> Result<ProtectionStatus, CommandError> {
    let (current, next) = (Zeroizing::new(current), Zeroizing::new(next));
    blocking(&svc, move |p| p.change_password(&current, &next)).await
}

#[tauri::command]
pub async fn verify_password(
    svc: Shared<'_>,
    password: String,
) -> Result<ProtectionStatus, CommandError> {
    let password = Zeroizing::new(password);
    blocking(&svc, move |p| p.verify_password(&password)).await
}

/// 휴지통 확인창이 뜰 수 있어 작업 스레드에서 실행한다.
#[tauri::command]
pub async fn delete_originals(
    svc: Shared<'_>,
    asset_ids: Vec<i64>,
) -> Result<OriginalsResult, CommandError> {
    blocking(&svc, move |p| p.delete_originals(&asset_ids)).await
}

#[tauri::command]
pub async fn move_originals(
    svc: Shared<'_>,
    asset_ids: Vec<i64>,
    dest_dir: String,
) -> Result<OriginalsResult, CommandError> {
    blocking(&svc, move |p| p.move_originals(&asset_ids, &dest_dir)).await
}

/// 백업 -> 표지 -> 서비스 정지 -> 재시작. 성공하면 앱이 다시 시작되므로 응답이 오지 않는다.
#[tauri::command]
pub async fn reset_managed_data(
    app: AppHandle,
    svc: Shared<'_>,
    input: ResetInput,
) -> Result<(), CommandError> {
    let password = input.password.map(Zeroizing::new);
    let forgot = input.forgot_password;
    blocking(&svc, move |p| p.prepare_reset(password.as_ref(), forgot)).await?;
    // 표지가 쓰였다 — 작업·감시 스레드와 DB 쓰기를 멈추고 다시 시작한다 (삭제는 새 프로세스가 DB 를 열기 전에 한다)
    blocking(&svc, |p| {
        p.ingest.shutdown();
        p.storage.pool.shutdown();
        Ok(())
    })
    .await?;
    app.restart()
}
