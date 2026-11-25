//! 이어보기 command: 마지막 재생 위치 조회·저장, 선택·폴더·전체 초기화, 관리 폴더별 통계. 로직은
//! `mam_core::userdata::UserData`(`resume.rs`)에 있고 여기서는 `spawn_blocking`·오류 변환만 한다.
//! 위치는 검색 색인 대상이 아니라 `search.wake()` 를 호출하지 않는다. 원본 미디어는 건드리지 않아 비밀번호 인증이 필요 없다.
//! 오류: `not_found`(없는 자산·관리 폴더), `invalid_input`(음수 위치).

use std::sync::Arc;

use mam_core::scan::IngestError;
use mam_core::userdata::{ResumePosition, ResumeSaveOutcome, ResumeStats, UserData};
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;

type Shared<'a> = State<'a, Arc<UserData>>;

async fn blocking<T, F>(svc: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

/// 자산의 마지막 재생 위치 (없으면 `null`).
#[tauri::command]
pub async fn resume_get(svc: Shared<'_>, asset_id: i64) -> Result<Option<ResumePosition>, CommandError> {
    blocking(&svc, move |u| u.resume_get(asset_id)).await
}

/// 위치 저장 + 판정 (`saved` / `cleared` / `skipped`).
#[tauri::command]
pub async fn resume_save(
    svc: Shared<'_>,
    asset_id: i64,
    position_ms: i64,
    duration_ms: Option<i64>,
) -> Result<ResumeSaveOutcome, CommandError> {
    blocking(&svc, move |u| u.resume_save(asset_id, position_ms, duration_ms)).await
}

/// 선택한 자산 중 기록이 있는 수.
#[tauri::command]
pub async fn resume_count_assets(svc: Shared<'_>, asset_ids: Vec<i64>) -> Result<i64, CommandError> {
    blocking(&svc, move |u| u.resume_count_assets(&asset_ids)).await
}

/// 선택한 자산의 기록 삭제 — 삭제 건수.
#[tauri::command]
pub async fn resume_clear_assets(svc: Shared<'_>, asset_ids: Vec<i64>) -> Result<i64, CommandError> {
    blocking(&svc, move |u| u.resume_clear_assets(&asset_ids)).await
}

/// 관리 폴더 범위의 기록 삭제 — 삭제 건수.
#[tauri::command]
pub async fn resume_clear_folder(svc: Shared<'_>, folder_id: i64) -> Result<i64, CommandError> {
    blocking(&svc, move |u| u.resume_clear_folder(folder_id)).await
}

/// 모든 기록 삭제 — 삭제 건수.
#[tauri::command]
pub async fn resume_clear_all(svc: Shared<'_>) -> Result<i64, CommandError> {
    blocking(&svc, |u| u.resume_clear_all()).await
}

/// 관리 폴더별 이어보기 사용 여부 설정 (기록은 유지). 없는·등록 해제 중인 폴더는 `not_found`.
#[tauri::command]
pub async fn resume_set_folder_enabled(svc: Shared<'_>, folder_id: i64, enabled: bool) -> Result<(), CommandError> {
    blocking(&svc, move |u| u.resume_set_folder_enabled(folder_id, enabled)).await
}

/// 관리 폴더별 기록 수(0건 포함) + 전체 행 수.
#[tauri::command]
pub async fn resume_stats(svc: Shared<'_>) -> Result<ResumeStats, CommandError> {
    blocking(&svc, |u| u.resume_stats()).await
}

#[cfg(test)]
#[path = "resume_commands_tests.rs"]
mod resume_commands_tests;
