//! 플러그인 작업 command 4개 (P5): 전체 실행·내보내기·작업 건수·실패 재시도. 로직은 `mam_plugin_runtime::PluginJobs` 에 있고
//! 여기서는 인자 검사·`spawn_blocking`·오류 변환만 한다. 오류 코드 표는 명세 §C3 (런타임 오류 변환은 동결된 `plugin_errors.rs`).

use std::sync::Arc;

use mam_plugin_runtime::jobs::{ExportError, PluginJobsError, RunAllError};
use mam_plugin_runtime::{ExportResult, InstallError, PluginJobs, PluginTaskCount, RunAllResult};
use tauri::State;

use crate::error::{CommandError, ErrorCode};

type Jobs<'a> = State<'a, Arc<PluginJobs>>;

/// `plugin_error` 메시지는 정적 i18n 키 — provider 가 계약 밖의 출력을 돌려준 경우
const BAD_OUTPUT_KEY: &str = "plugin.error.protocol";

async fn blocking<T, F>(f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| {
        tracing::error!(%e, "blocking task failed");
        CommandError::internal()
    })?
}

/// 플러그인·provider ID 인자 — 비었거나 NUL 이 있으면 `invalid_input` (없는 ID 는 런타임이 `not_found`)
fn id_arg(value: &str) -> Result<(), CommandError> {
    if value.is_empty() || value.contains('\0') {
        return Err(CommandError::new(ErrorCode::InvalidInput, "invalid plugin or provider id"));
    }
    Ok(())
}

/// 내보내기 금지 경로 계산 — MAMData 루트와 **현재 설정의** 백업 폴더. 호출마다 다시 읽으므로 실행 중 백업 폴더를 바꿔도 보호된다.
/// 설정을 읽지 못하면 기본 백업 폴더를 쓴다 (보호 목록을 비우지 않는다)
pub(crate) fn plugin_protected_source(storage: Arc<crate::storage_commands::Storage>, root: std::path::PathBuf) -> mam_plugin_runtime::jobs::ProtectedSource {
    Arc::new(move || {
        let backups = storage.app_paths().map_or_else(|_| root.join("backups"), |p| std::path::PathBuf::from(p.backups_dir));
        vec![root.clone(), backups]
    })
}

pub(crate) fn run_all_error(e: RunAllError) -> CommandError {
    match e {
        RunAllError::Run(e) => e.into(),
        RunAllError::ExportProvider => CommandError::new(ErrorCode::InvalidInput, "export providers cannot be run on all assets"),
        RunAllError::Spawn(e) => CommandError::logged(ErrorCode::Internal, "cannot start the run-all task", &e),
    }
}

pub(crate) fn export_error(e: ExportError) -> CommandError {
    match e {
        ExportError::InvalidInput(m) => CommandError::new(ErrorCode::InvalidInput, m),
        ExportError::Run(e) => e.into(),
        ExportError::BadOutput(m) => {
            tracing::warn!(reason = m, "export provider returned an invalid output");
            CommandError::new(ErrorCode::PluginError, BAD_OUTPUT_KEY)
        }
        // `io` / `disk_full` 분류는 설치 오류 변환과 같다
        ExportError::Io(e) => InstallError::Io(e).into(),
        ExportError::Db(e) => e.into(),
        ExportError::Folders(m) => CommandError::logged(ErrorCode::Internal, "cannot verify the destination", &m),
    }
}

pub(crate) fn retry_error(e: PluginJobsError) -> CommandError {
    match e {
        PluginJobsError::NotInstalled => CommandError::new(ErrorCode::NotFound, "plugin not installed"),
        PluginJobsError::Db(e) => e.into(),
    }
}

/// provider 를 모든 활성 자산에 등록한다 (백그라운드). `started=false` = 이미 실행 중
#[tauri::command]
pub async fn plugin_run_all(jobs: Jobs<'_>, plugin_id: String, provider_id: String) -> Result<RunAllResult, CommandError> {
    id_arg(&plugin_id)?;
    id_arg(&provider_id)?;
    let jobs = Arc::clone(&jobs);
    blocking(move || jobs.run_all(&plugin_id, &provider_id).map_err(run_all_error)).await
}

/// 내보내기 provider 호출 결과를 `dest_path` 에 쓴다 — 관리 폴더·앱 데이터와 겹치는 경로는 provider 호출 전에 거부한다
#[tauri::command]
pub async fn plugin_export(
    jobs: Jobs<'_>,
    plugin_id: String,
    provider_id: String,
    asset_ids: Vec<i64>,
    dest_path: String,
) -> Result<ExportResult, CommandError> {
    id_arg(&plugin_id)?;
    id_arg(&provider_id)?;
    let jobs = Arc::clone(&jobs);
    blocking(move || jobs.export(&plugin_id, &provider_id, asset_ids, &dest_path).map_err(export_error)).await
}

#[tauri::command]
pub async fn plugin_task_counts(jobs: Jobs<'_>) -> Result<Vec<PluginTaskCount>, CommandError> {
    let jobs = Arc::clone(&jobs);
    blocking(move || jobs.task_counts().map_err(CommandError::from)).await
}

/// 실패한 작업을 다시 대기로 — 되돌린 자산 수
#[tauri::command]
pub async fn plugin_retry_failed(jobs: Jobs<'_>, plugin_id: String) -> Result<u64, CommandError> {
    id_arg(&plugin_id)?;
    let jobs = Arc::clone(&jobs);
    blocking(move || jobs.retry_failed(&plugin_id).map_err(retry_error)).await
}

#[cfg(test)]
#[path = "plugin_job_commands_tests.rs"]
mod plugin_job_commands_tests;
