//! 폴더별 변경 감지 방식·재스캔 정책 command. 로직·검증은 `mam_core::ingest` (`detection.rs`) 에 있다.
//! 인자는 `serde_json::Value` 로 받아 코어가 검증한다 — Tauri 의 인자 역직렬화 오류는 계약 오류 형태가 아니기 때문이다.
//! Rust <-> TS 계약: `src/lib/ipc-contract-watch.ts`.

use std::sync::Arc;

use mam_core::ingest::Ingest;
use mam_core::scan::ManagedFolderView;
use mam_core::watch::ChangeDetectorInfo;
use serde_json::Value;
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;

/// 이 폴더 기준 감지기 목록 (사용 가능 여부·사유 키·옵션 필드). 없는 폴더는 `not_found`.
#[tauri::command]
pub async fn list_change_detectors(
    ingest: State<'_, Arc<Ingest>>,
    folder_id: i64,
) -> Result<Vec<ChangeDetectorInfo>, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.list_change_detectors(folder_id)).await
}

/// 변경 감지 방식 저장 + 감시 재설정. `mode` null = 자동. 등록되지 않은 방식·잘못된 옵션은 `invalid_input`.
#[tauri::command]
pub async fn set_folder_watch_mode(
    ingest: State<'_, Arc<Ingest>>,
    folder_id: i64,
    mode: Option<String>,
    options: Option<Value>,
) -> Result<ManagedFolderView, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.set_folder_watch_mode(folder_id, mode, options)).await
}

/// 재스캔 정책 저장. `policy` null = 전역 기본. 잘못된 정책은 `invalid_input` (메시지에 필드 경로).
#[tauri::command]
pub async fn set_folder_rescan_policy(
    ingest: State<'_, Arc<Ingest>>,
    folder_id: i64,
    policy: Option<Value>,
) -> Result<ManagedFolderView, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.set_folder_rescan_policy(folder_id, policy)).await
}

#[cfg(test)]
#[path = "folder_detection_commands_tests.rs"]
mod tests;
