//! 검색 command (P2-5): 검색·탐색, 자산 상세, 최근 검색. 로직은 `search_service::SearchService`(조정기)와
//! `mam_core::query::QueryService`(SQLite) 에 있고 여기서는 `spawn_blocking`·오류 변환만 한다.
//! 계약 정본: UI 계약 `ipc-contract.ts` (P2-5), TS 쪽 `src/lib/ipc.ts`.

use std::sync::Arc;

use mam_core::query::{
    AssetDetail, QueryService, RecentSearch, SearchFilters, SearchIndex, SearchIndexRequest,
    SearchPage, SearchRequest,
};
use mam_core::scan::IngestError;
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;
use crate::search_hybrid::{SimilarPage, SimilarRequest};
use crate::search_service::SearchService;

type Query<'a> = State<'a, Arc<QueryService>>;

async fn blocking<T, F>(svc: &Query<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&QueryService) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

#[tauri::command]
pub async fn search_assets(
    search: State<'_, Arc<SearchService>>,
    req: SearchRequest,
) -> Result<SearchPage, CommandError> {
    let search = Arc::clone(&search);
    tauri::async_runtime::spawn_blocking(move || search.search(&req))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_asset_detail(svc: Query<'_>, asset_id: i64) -> Result<AssetDetail, CommandError> {
    blocking(&svc, move |q| q.detail(asset_id)).await
}

#[tauri::command]
pub async fn list_recent_searches(svc: Query<'_>) -> Result<Vec<RecentSearch>, CommandError> {
    blocking(&svc, |q| q.list_recent()).await
}

#[tauri::command]
pub async fn record_recent_search(
    svc: Query<'_>,
    query: String,
    filters: SearchFilters,
) -> Result<(), CommandError> {
    blocking(&svc, move |q| q.record_recent(&query, &filters)).await
}

#[tauri::command]
pub async fn clear_recent_searches(svc: Query<'_>) -> Result<(), CommandError> {
    blocking(&svc, |q| q.clear_recent()).await
}

/// 유사 미디어 (P3) — 시각 벡터 활성 인덱스에서 기준 자산과 비슷한 자산. 자신·같은 해시 중복본은 제외한다.
#[tauri::command]
pub async fn find_similar(
    search: State<'_, Arc<SearchService>>,
    req: SimilarRequest,
) -> Result<SimilarPage, CommandError> {
    let search = Arc::clone(&search);
    tauri::async_runtime::spawn_blocking(move || search.find_similar(&req))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

/// 목차 레일·행 모델용 구간 분포 — 질의·필터·정렬은 `search_assets` 와 같고 limit·cursor 는 없다.
#[tauri::command]
pub async fn search_index(
    search: State<'_, Arc<SearchService>>,
    req: SearchIndexRequest,
) -> Result<SearchIndex, CommandError> {
    let search = Arc::clone(&search);
    tauri::async_runtime::spawn_blocking(move || search.search_index(&req))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}
