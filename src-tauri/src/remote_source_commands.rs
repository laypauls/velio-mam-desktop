//! 원격 스토리지 접속 command — 목록·생성·수정·삭제·접속 시험. 로직·검증은 `mam_core::ingest` (`remote_api.rs`) 에 있다.
//! 비밀(`input.secret`)은 인자로만 받고 어디에도 돌려주지 않는다 — 응답은 `hasSecret` 만 싣는다. 오류 문구에는 필드 이름만 간다.
//! Rust <-> TS 계약: `src/lib/ipc-contract-remote.ts`.

use std::sync::Arc;

use mam_core::ingest::{Ingest, RemoteSourceInput, RemoteTestResult};
use mam_core::watch::remote::SourceView;
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;

/// 저장된 접속 목록 (이름 순, 비밀 없음).
#[tauri::command]
pub async fn list_remote_sources(ingest: State<'_, Arc<Ingest>>) -> Result<Vec<SourceView>, CommandError> {
    run_blocking(Arc::clone(&ingest), |i| i.list_remote_sources()).await
}

/// 접속 생성 — `secret` 필수. 이름 중복은 `already_registered`, 잘못된 값은 `invalid_input`(필드 이름).
#[tauri::command]
pub async fn create_remote_source(ingest: State<'_, Arc<Ingest>>, input: RemoteSourceInput) -> Result<SourceView, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.create_remote_source(input)).await
}

/// 접속 수정 — `secret` 이 없으면 저장된 비밀을 유지한다. 쓰는 폴더에 닿지 않는 경로 변환은 `invalid_input`(`pathMap`).
#[tauri::command]
pub async fn update_remote_source(
    ingest: State<'_, Arc<Ingest>>,
    source_id: i64,
    input: RemoteSourceInput,
) -> Result<SourceView, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.update_remote_source(source_id, input)).await
}

/// 접속 삭제 — 쓰는 폴더가 있으면 `busy`, 없는 접속은 `not_found`.
#[tauri::command]
pub async fn delete_remote_source(ingest: State<'_, Arc<Ingest>>, source_id: i64) -> Result<(), CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.delete_remote_source(source_id)).await
}

/// 접속 시험 (네트워크 사용 — 사용자가 누른 경우에만). 연결·인증·지문 문제는 오류가 아니라 결과(`errorKey`)로 돌아온다.
/// 호스트 키 지문이 없으면 인증하지 않고 지문만 돌려준다.
#[tauri::command]
pub async fn test_remote_source(
    ingest: State<'_, Arc<Ingest>>,
    source_id: Option<i64>,
    input: RemoteSourceInput,
) -> Result<RemoteTestResult, CommandError> {
    run_blocking(Arc::clone(&ingest), move |i| i.test_remote_source(source_id, input)).await
}

#[cfg(test)]
#[path = "remote_source_commands_tests.rs"]
mod tests;
