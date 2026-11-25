//! AI command (P3): 설정·동의·키·로컬 모델·분석 작업·결과 조회. 계약 정본 `src/lib/ipc-contract-ai.ts`, TS 쪽 `src/lib/ipc.ts`.
//! 로직은 `ai::AiService` 에 있고 여기서는 `spawn_blocking`·오류 변환만 한다. API 키는 인자로만 받고 어디에도 되돌려주지 않는다.

use std::sync::Arc;

use mam_ai::external::ProviderCheck;
use mam_ai::external::cli::LoginState;
use mam_core::ai_jobs::QueueResult;
use mam_core::ai_store::{AiUsageRow, AnalysisRun, AnalysisView};
use tauri::State;

use crate::ai::service::{AssetAi, EditInput, ProviderStatus};
use crate::ai::{AiFailure, AiService, AiSettings};
use crate::error::CommandError;
use crate::search_service::SearchService;

type Svc<'a> = State<'a, Arc<AiService>>;

async fn blocking<T, F>(svc: &Svc<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&AiService) -> Result<T, AiFailure> + Send + 'static,
{
    let svc = Arc::clone(svc);
    tauri::async_runtime::spawn_blocking(move || f(&svc))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_ai_settings(svc: Svc<'_>) -> Result<AiSettings, CommandError> {
    blocking(&svc, |s| Ok(s.get_settings())).await
}

/// `patch` 는 `AiSettingsPatch` — 형식 오류도 `invalid_input` 으로 돌려주려고 원시 JSON 으로 받는다.
#[tauri::command]
pub async fn update_ai_settings(
    svc: Svc<'_>,
    patch: serde_json::Value,
) -> Result<AiSettings, CommandError> {
    blocking(&svc, move |s| s.update_settings(patch)).await
}

#[tauri::command]
pub async fn give_ai_consent(svc: Svc<'_>) -> Result<AiSettings, CommandError> {
    blocking(&svc, |s| s.give_consent()).await
}

#[tauri::command]
pub async fn revoke_ai_consent(svc: Svc<'_>) -> Result<AiSettings, CommandError> {
    blocking(&svc, |s| s.revoke_consent()).await
}

#[tauri::command]
pub async fn list_ai_providers(svc: Svc<'_>) -> Result<Vec<ProviderStatus>, CommandError> {
    blocking(&svc, |s| Ok(s.list_providers())).await
}

#[tauri::command]
pub async fn set_api_key(
    svc: Svc<'_>,
    provider: String,
    key: String,
    persist: bool,
) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.set_api_key(&provider, &key, persist)).await
}

#[tauri::command]
pub async fn clear_api_key(svc: Svc<'_>, provider: String) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.clear_api_key(&provider)).await
}

#[tauri::command]
pub async fn set_provider_model(
    svc: Svc<'_>,
    provider: String,
    model: Option<String>,
) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.set_provider_model(&provider, model)).await
}

#[tauri::command]
pub async fn check_provider(svc: Svc<'_>, provider: String) -> Result<ProviderCheck, CommandError> {
    blocking(&svc, move |s| s.check_provider(&provider)).await
}

/// 인증 방식 전환 (`apiKey` | `subscription`). 구독은 claude 만 — 그 외는 `invalid_input`.
#[tauri::command]
pub async fn set_provider_auth_mode(
    svc: Svc<'_>,
    provider: String,
    mode: String,
) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.set_provider_auth_mode(&provider, &mode)).await
}

/// CLI 설치·버전·로그인 상태 재확인 — 네트워크 요청 없음, 동의 전 허용.
#[tauri::command]
pub async fn detect_ai_cli(svc: Svc<'_>, provider: String) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.detect_ai_cli(&provider)).await
}

/// 로그인 명령 실행 -> CLI 출력의 https 인증 URL 을 시스템 기본 브라우저로 연다 (URL 은 프론트가 지정할 수 없다).
#[tauri::command]
pub async fn ai_cli_login_start(
    svc: Svc<'_>,
    provider: String,
) -> Result<LoginState, CommandError> {
    blocking(&svc, move |s| s.cli_login_start(&provider)).await
}

#[tauri::command]
pub async fn ai_cli_login_status(
    svc: Svc<'_>,
    provider: String,
) -> Result<LoginState, CommandError> {
    blocking(&svc, move |s| s.cli_login_status(&provider)).await
}

#[tauri::command]
pub async fn ai_cli_login_cancel(
    svc: Svc<'_>,
    provider: String,
) -> Result<LoginState, CommandError> {
    blocking(&svc, move |s| s.cli_login_cancel(&provider)).await
}

/// 캡처해 둔 인증 URL 만 다시 연다 (URL 인자 없음).
#[tauri::command]
pub async fn open_ai_cli_login_url(svc: Svc<'_>, provider: String) -> Result<bool, CommandError> {
    blocking(&svc, move |s| s.open_cli_login_url(&provider)).await
}

/// 진행 중인 외부 AI 요청이 있으면 `ai_busy`.
#[tauri::command]
pub async fn ai_cli_logout(svc: Svc<'_>, provider: String) -> Result<ProviderStatus, CommandError> {
    blocking(&svc, move |s| s.cli_logout(&provider)).await
}

#[tauri::command]
pub async fn list_local_models(
    svc: Svc<'_>,
) -> Result<Vec<crate::ai::models::LocalModel>, CommandError> {
    blocking(&svc, |s| Ok(s.list_models())).await
}

/// 다운로드를 시작하고 바로 돌아온다 — 진행률은 `models://progress`.
#[tauri::command]
pub async fn download_model(svc: Svc<'_>, model_id: String) -> Result<(), CommandError> {
    blocking(&svc, move |s| s.download_model(&model_id)).await
}

#[tauri::command]
pub async fn analyze_assets(
    svc: Svc<'_>,
    asset_ids: Vec<i64>,
    kinds: Option<Vec<String>>,
    force: bool,
) -> Result<QueueResult, CommandError> {
    blocking(&svc, move |s| {
        s.analyze_assets(&asset_ids, kinds.as_deref(), force)
    })
    .await
}

#[tauri::command]
pub async fn analyze_all(svc: Svc<'_>, only_unanalyzed: bool) -> Result<QueueResult, CommandError> {
    blocking(&svc, move |s| s.analyze_all(only_unanalyzed)).await
}

#[tauri::command]
pub async fn retry_failed_analysis(
    svc: Svc<'_>,
    asset_ids: Option<Vec<i64>>,
) -> Result<QueueResult, CommandError> {
    blocking(&svc, move |s| s.retry_failed(asset_ids.as_deref())).await
}

#[tauri::command]
pub async fn get_asset_ai(svc: Svc<'_>, asset_id: i64) -> Result<AssetAi, CommandError> {
    blocking(&svc, move |s| s.get_asset_ai(asset_id)).await
}

#[tauri::command]
pub async fn update_analysis(
    svc: Svc<'_>,
    search: State<'_, Arc<SearchService>>,
    asset_id: i64,
    edit: EditInput,
) -> Result<AnalysisView, CommandError> {
    let out = blocking(&svc, move |s| s.edit_analysis(asset_id, &edit)).await;
    search.wake(); // 사용자 편집은 priority 0 — 색인 워커를 바로 깨운다
    out
}

#[tauri::command]
pub async fn list_analysis_history(
    svc: Svc<'_>,
    asset_id: i64,
) -> Result<Vec<AnalysisRun>, CommandError> {
    blocking(&svc, move |s| s.list_history(asset_id)).await
}

#[tauri::command]
pub async fn list_ai_usage(
    svc: Svc<'_>,
    from_day: String,
    to_day: String,
) -> Result<Vec<AiUsageRow>, CommandError> {
    blocking(&svc, move |s| s.list_usage(&from_day, &to_day)).await
}
