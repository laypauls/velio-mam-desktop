//! 플러그인 설치·관리 command 8개 (P6). 로직은 `mam_plugin_runtime::PluginRuntime` 에 있고 여기서는 인자 검증·로케일 읽기·
//! `spawn_blocking`·오류 변환(`plugin_errors.rs`)만 한다. 설치·제거·워커 호출은 블로킹이라 모든 본문이 `spawn_blocking` 에서 돈다.
//!
//! 오류 코드 표는 명세 §C3. 로케일은 command 가 설정 `language` 에서 읽는다(표시 언어 → `en` → id 순으로 번역을 찾는다).

use std::path::PathBuf;
use std::sync::Arc;

use mam_plugin_runtime::{PluginCommandResult, PluginContributions, PluginInfo, PluginPreview, PluginRuntime};
use tauri::State;

use crate::error::{CommandError, ErrorCode};
use crate::storage_commands::Storage;

type Runtime<'a> = State<'a, Arc<PluginRuntime>>;
type Settings<'a> = State<'a, Arc<Storage>>;

/// 표시 언어 코드 — 설정을 읽지 못하면 `en`
fn language(storage: &Storage) -> String {
    storage.get_settings().map_or_else(|_| "en".to_owned(), |v| v.settings.language)
}

/// 사용자가 고른 폴더 경로 문자열 -> `PathBuf`. 비었거나 NUL 이 있으면 `invalid_input`
fn dir_arg(dir: String) -> Result<PathBuf, CommandError> {
    if dir.is_empty() || dir.contains('\0') {
        return Err(CommandError::new(ErrorCode::InvalidInput, "invalid path"));
    }
    Ok(PathBuf::from(dir))
}

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

#[tauri::command]
pub async fn plugin_list(runtime: Runtime<'_>, storage: Settings<'_>) -> Result<Vec<PluginInfo>, CommandError> {
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.list(&language(&st)))).await
}

/// 설치 전 미리보기 — 폴더는 읽기만 한다
#[tauri::command]
pub async fn plugin_inspect_dir(runtime: Runtime<'_>, storage: Settings<'_>, dir: String) -> Result<PluginPreview, CommandError> {
    let dir = dir_arg(dir)?;
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.inspect_dir(&dir, &language(&st))?)).await
}

/// 설치·업그레이드. `expected_*` 는 미리보기에서 사용자가 본 값 — 그 사이 폴더가 바뀌었으면 `invalid_input`
#[tauri::command]
pub async fn plugin_install(
    runtime: Runtime<'_>,
    storage: Settings<'_>,
    dir: String,
    expected_id: String,
    expected_version: String,
    granted_permissions: Vec<String>,
) -> Result<PluginInfo, CommandError> {
    let dir = dir_arg(dir)?;
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.install(&dir, &expected_id, &expected_version, granted_permissions, &language(&st))?)).await
}

#[tauri::command]
pub async fn plugin_set_enabled(runtime: Runtime<'_>, storage: Settings<'_>, plugin_id: String, enabled: bool) -> Result<PluginInfo, CommandError> {
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.set_enabled(&plugin_id, enabled, &language(&st))?)).await
}

#[tauri::command]
pub async fn plugin_grant_permissions(
    runtime: Runtime<'_>,
    storage: Settings<'_>,
    plugin_id: String,
    permissions: Vec<String>,
) -> Result<PluginInfo, CommandError> {
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.grant_permissions(&plugin_id, permissions, &language(&st))?)).await
}

/// 제거. `keep_data=false` 면 그 플러그인의 KV·작업 행도 지운다(배치) — 원본 미디어는 건드리지 않는다
#[tauri::command]
pub async fn plugin_uninstall(runtime: Runtime<'_>, plugin_id: String, keep_data: bool) -> Result<(), CommandError> {
    let rt = Arc::clone(&runtime);
    blocking(move || Ok(rt.uninstall(&plugin_id, keep_data)?)).await
}

/// 메뉴 명령 실행 — 선택 자산 0..=10,000개(중복 제거). 결과는 사용자에게 보일 짧은 메시지 또는 `null`
#[tauri::command]
pub async fn plugin_run_command(
    runtime: Runtime<'_>,
    plugin_id: String,
    command_id: String,
    asset_ids: Vec<i64>,
) -> Result<PluginCommandResult, CommandError> {
    let rt = Arc::clone(&runtime);
    blocking(move || Ok(rt.run_command(&plugin_id, &command_id, asset_ids)?)).await
}

/// 활성 플러그인의 메뉴 항목·내보내기 provider (번역된 title, `when` 은 원문)
#[tauri::command]
pub async fn plugin_contributions(runtime: Runtime<'_>, storage: Settings<'_>) -> Result<PluginContributions, CommandError> {
    let (rt, st) = (Arc::clone(&runtime), Arc::clone(&storage));
    blocking(move || Ok(rt.contributions(&language(&st)))).await
}

#[cfg(test)]
mod tests {
    // Categories: Validation (경로 인자)
    use super::*;

    #[test]
    fn t_ipc_plugin_00_dir_arg_rejects_empty_and_nul_paths() {
        assert_eq!(dir_arg(String::new()).unwrap_err().code(), ErrorCode::InvalidInput);
        assert_eq!(dir_arg("C:\\x\0y".into()).unwrap_err().code(), ErrorCode::InvalidInput);
        assert_eq!(dir_arg("C:\\plugins\\demo".into()).unwrap(), PathBuf::from("C:\\plugins\\demo"));
    }
}

#[cfg(test)]
#[path = "plugin_commands_tests.rs"]
mod contract_tests;
