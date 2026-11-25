//! 사용자 데이터 command (P2-7): 태그·메모·즐겨찾기·컬렉션. 로직은 `mam_core::userdata::UserData` 에 있고
//! 여기서는 인자 변환·`spawn_blocking`·오류 변환만 한다. 계약 정본: UI 계약 `ipc-contract.ts` (P2-7), TS 쪽 `src/lib/ipc.ts`.

use std::sync::Arc;

use mam_core::scan::IngestError;
use mam_core::userdata::{
    CollectionMembership, CollectionView, PlayQueueItem, ReorderTarget, TagView, UpdateTagsInput,
    UserData,
};
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;
use crate::search_service::SearchService;

type Shared<'a> = State<'a, Arc<UserData>>;
/// 사용자 편집 뒤 색인 워커를 깨워 priority 0 이벤트를 바로 처리하게 한다 (P2-5 D9)
type Wake<'a> = State<'a, Arc<SearchService>>;

async fn blocking<T, F>(svc: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

#[tauri::command]
pub async fn list_tags(svc: Shared<'_>) -> Result<Vec<TagView>, CommandError> {
    blocking(&svc, |u| u.list_tags()).await
}

#[tauri::command]
pub async fn update_asset_tags(
    svc: Shared<'_>,
    search: Wake<'_>,
    input: UpdateTagsInput,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.update_asset_tags(input)).await;
    search.wake();
    out
}

#[tauri::command]
pub async fn rename_tag(
    svc: Shared<'_>,
    search: Wake<'_>,
    tag_id: i64,
    name: String,
) -> Result<TagView, CommandError> {
    let out = blocking(&svc, move |u| u.rename_tag(tag_id, &name)).await;
    search.wake();
    out
}

#[tauri::command]
pub async fn delete_tag(
    svc: Shared<'_>,
    search: Wake<'_>,
    tag_id: i64,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.delete_tag(tag_id)).await;
    search.wake();
    out
}

#[tauri::command]
pub async fn set_memo(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_id: i64,
    memo: String,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.set_memo(asset_id, &memo)).await;
    search.wake();
    out
}

/// 콘텐츠 제목 변경 (`title` null/공백 = 지우기). 설정 `titleEditEnabled` 가 꺼져 있으면 `invalid_input`,
/// 없는 자산은 `not_found`.
#[tauri::command]
pub async fn set_asset_title(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_id: i64,
    title: Option<String>,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.set_title(asset_id, title.as_deref())).await;
    search.wake();
    out
}

#[tauri::command]
pub async fn set_favorite(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_ids: Vec<i64>,
    favorite: bool,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.set_favorite(&asset_ids, favorite)).await;
    search.wake();
    out
}

/// 별점 설정 (0 = 지우기). 0..=5 밖이면 `invalid_input`, 없는 자산이 섞여 있으면 `not_found`(전체 롤백).
#[tauri::command]
pub async fn set_rating(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_ids: Vec<i64>,
    rating: i64,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| {
        let rating = u8::try_from(rating).map_err(|_| IngestError::InvalidInput("rating"))?;
        u.set_rating(&asset_ids, rating)
    })
    .await;
    search.wake();
    out
}

/// 사이드바 별점 개수: 길이 6, 인덱스 = 별점 (0 = 미평가). 활성 자산 중 파일 없음 제외, 오프라인 포함.
#[tauri::command]
pub async fn rating_counts(svc: Shared<'_>) -> Result<Vec<u64>, CommandError> {
    blocking(&svc, |u| u.rating_counts().map(|c| c.to_vec())).await
}

#[tauri::command]
pub async fn list_collections(svc: Shared<'_>) -> Result<Vec<CollectionView>, CommandError> {
    blocking(&svc, |u| u.list_collections()).await
}

#[tauri::command]
pub async fn create_collection(
    svc: Shared<'_>,
    name: String,
) -> Result<CollectionView, CommandError> {
    blocking(&svc, move |u| u.create_collection(&name)).await
}

#[tauri::command]
pub async fn rename_collection(
    svc: Shared<'_>,
    collection_id: i64,
    name: String,
) -> Result<CollectionView, CommandError> {
    blocking(&svc, move |u| u.rename_collection(collection_id, &name)).await
}

#[tauri::command]
pub async fn delete_collection(
    svc: Shared<'_>,
    search: Wake<'_>,
    collection_id: i64,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| u.delete_collection(collection_id)).await;
    search.wake();
    out
}

#[tauri::command]
pub async fn add_to_collection(
    svc: Shared<'_>,
    search: Wake<'_>,
    collection_id: i64,
    asset_ids: Vec<i64>,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| {
        u.add_to_collection(collection_id, &asset_ids)
    })
    .await;
    search.wake();
    out
}

#[tauri::command]
pub async fn remove_from_collection(
    svc: Shared<'_>,
    search: Wake<'_>,
    collection_id: i64,
    asset_ids: Vec<i64>,
) -> Result<(), CommandError> {
    let out = blocking(&svc, move |u| {
        u.remove_from_collection(collection_id, &asset_ids)
    })
    .await;
    search.wake();
    out
}

/// 선택 항목을 한 덩어리로 `target`(`{kind:"before",assetId}|{kind:"start"}|{kind:"end"}`)으로 옮긴다.
/// 순서는 검색 색인 대상이 아니므로 색인 워커를 깨우지 않는다. 오류: `not_found`(컬렉션 없음), `invalid_input`.
#[tauri::command]
pub async fn reorder_collection(
    svc: Shared<'_>,
    collection_id: i64,
    asset_ids: Vec<i64>,
    target: ReorderTarget,
) -> Result<(), CommandError> {
    blocking(&svc, move |u| {
        u.reorder_collection(collection_id, &asset_ids, target)
    })
    .await
}

/// 선택 자산들의 컬렉션별 소속 수 (0 인 컬렉션은 응답에 없다). 오류: `invalid_input`(1만 건 초과).
#[tauri::command]
pub async fn collection_membership(
    svc: Shared<'_>,
    asset_ids: Vec<i64>,
) -> Result<Vec<CollectionMembership>, CommandError> {
    blocking(&svc, move |u| u.collection_membership(&asset_ids)).await
}

/// 연속 재생 대기열 — 컬렉션 순서 전체(오프라인·파일 없음 포함). 오류: `not_found`.
#[tauri::command]
pub async fn collection_play_queue(
    svc: Shared<'_>,
    collection_id: i64,
) -> Result<Vec<PlayQueueItem>, CommandError> {
    blocking(&svc, move |u| u.collection_play_queue(collection_id)).await
}

#[cfg(test)]
#[path = "collection_commands_tests.rs"]
mod collection_commands_tests;
