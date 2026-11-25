//! 사용자 정의 메타데이터 필드 command (14개). 로직은 `mam_core::userdata::UserData`(`custom_fields/`)에 있고
//! 여기서는 인자 변환·`spawn_blocking`·오류 변환만 한다. 계약: `src/lib/ipc-contract-metadata.ts`, 래퍼 `ipc-metadata.ts`.
//!
//! - 값을 바꾸는 command 는 끝난 뒤 색인 워커를 깨운다 (priority 0 이벤트 즉시 처리).
//! - 필드·선택지 삭제는 짧은 트랜잭션으로 "삭제 중" 표시만 하고 즉시 돌아온다. 값 정리(5,000 자산 청크)는
//!   별도 스레드가 이어서 하며(`kick_purge`) command 는 기다리지 않는다. 앱 시작 때도 남은 삭제를 이어서 한다.
//! - 오류: `invalid_input`(이름·값·타입 불일치·20개 초과·자산 1만 건 초과), `not_found`(없거나 삭제 중인 필드·선택지·자산),
//!   `already_registered`(같은 이름의 필드·같은 필드 안 선택지).

use std::sync::Arc;
use std::thread::JoinHandle;

use mam_core::scan::IngestError;
use mam_core::userdata::{UserData, ValueType};
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;
use crate::metadata_types::{
    AssetFieldValueView, FieldOptionView, FieldPatchDto, MetadataFieldView, to_patches,
};
use crate::search_service::SearchService;

type Shared<'a> = State<'a, Arc<UserData>>;
type Wake<'a> = State<'a, Arc<SearchService>>;

async fn blocking<T, F>(svc: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

/// 삭제 요청 뒤 남은 값 정리를 별도 스레드에서 시작한다 (기다리지 않음). 끝나면 색인 워커를 깨운다.
/// 반환 핸들은 테스트가 끝을 기다리는 데만 쓴다. 스레드를 못 만들면 다음 앱 시작 때 이어서 처리된다.
pub(crate) fn kick_purge(user: Arc<UserData>, search: Option<Arc<SearchService>>) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("mam-umeta-purge".into())
        .spawn(move || {
            if let Err(e) = user.purge_pending_metadata_deletions() {
                tracing::warn!(%e, "custom metadata purge failed; it resumes on the next start");
            }
            if let Some(s) = search {
                s.wake();
            }
        })
        .map_err(|e| tracing::warn!(%e, "cannot start the custom metadata purge"))
        .ok()
}

#[tauri::command]
pub async fn list_metadata_fields(svc: Shared<'_>) -> Result<Vec<MetadataFieldView>, CommandError> {
    blocking(&svc, |u| Ok(u.list_metadata_fields()?.into_iter().map(Into::into).collect())).await
}

/// `options` 는 enum/multiEnum 에서만 허용(그 외 타입에 비어 있지 않으면 `invalid_input`).
#[tauri::command]
pub async fn create_metadata_field(
    svc: Shared<'_>,
    name: String,
    value_type: ValueType,
    options: Vec<String>,
) -> Result<MetadataFieldView, CommandError> {
    blocking(&svc, move |u| u.create_metadata_field(&name, value_type, &options).map(Into::into)).await
}

/// 이름만 바꾼다 — 재색인 없음.
#[tauri::command]
pub async fn rename_metadata_field(
    svc: Shared<'_>,
    field_id: i64,
    name: String,
) -> Result<MetadataFieldView, CommandError> {
    blocking(&svc, move |u| u.rename_metadata_field(field_id, &name).map(Into::into)).await
}

/// `field_ids` 는 현재 필드 전체의 새 순서(누락·중복·모르는 id 는 `invalid_input`).
#[tauri::command]
pub async fn reorder_metadata_fields(svc: Shared<'_>, field_ids: Vec<i64>) -> Result<(), CommandError> {
    blocking(&svc, move |u| u.reorder_metadata_fields(&field_ids)).await
}

/// 필드 삭제 요청. 값을 가진 자산 수를 돌려주고 정리는 백그라운드에서 이어진다. 복원 불가.
#[tauri::command]
pub async fn delete_metadata_field(
    svc: Shared<'_>,
    search: Wake<'_>,
    field_id: i64,
) -> Result<u64, CommandError> {
    let affected = blocking(&svc, move |u| u.delete_metadata_field(field_id)).await?;
    kick_purge(Arc::clone(&svc), Some(Arc::clone(&search)));
    Ok(affected as u64)
}

/// 이 필드에 값을 가진 자산 수 (삭제 확인창용)
#[tauri::command]
pub async fn metadata_field_value_count(svc: Shared<'_>, field_id: i64) -> Result<i64, CommandError> {
    blocking(&svc, move |u| u.metadata_field_asset_count(field_id)).await
}

#[tauri::command]
pub async fn add_metadata_option(
    svc: Shared<'_>,
    field_id: i64,
    label: String,
) -> Result<FieldOptionView, CommandError> {
    blocking(&svc, move |u| u.add_metadata_option(field_id, &label).map(Into::into)).await
}

/// 이름만 바꾼다 — 재색인 없음 (색인에는 optionId 만 들어간다).
#[tauri::command]
pub async fn rename_metadata_option(
    svc: Shared<'_>,
    option_id: i64,
    label: String,
) -> Result<FieldOptionView, CommandError> {
    blocking(&svc, move |u| u.rename_metadata_option(option_id, &label).map(Into::into)).await
}

#[tauri::command]
pub async fn reorder_metadata_options(
    svc: Shared<'_>,
    field_id: i64,
    option_ids: Vec<i64>,
) -> Result<(), CommandError> {
    blocking(&svc, move |u| u.reorder_metadata_options(field_id, &option_ids)).await
}

/// 선택지 삭제 요청. 값을 가진 자산 수를 돌려주고 값 정리·재색인은 백그라운드에서 이어진다.
#[tauri::command]
pub async fn delete_metadata_option(
    svc: Shared<'_>,
    search: Wake<'_>,
    option_id: i64,
) -> Result<u64, CommandError> {
    let affected = blocking(&svc, move |u| u.delete_metadata_option(option_id)).await?;
    kick_purge(Arc::clone(&svc), Some(Arc::clone(&search)));
    Ok(affected as u64)
}

/// 이 선택지를 값으로 가진 자산 수 (삭제 확인창용)
#[tauri::command]
pub async fn metadata_option_usage(svc: Shared<'_>, option_id: i64) -> Result<i64, CommandError> {
    blocking(&svc, move |u| u.metadata_option_usage(option_id)).await
}

/// 자산 하나의 값 (필드 표시 순서, 값 없는 필드는 없음). 빈 목록일 수 있다.
#[tauri::command]
pub async fn get_asset_metadata(
    svc: Shared<'_>,
    asset_id: i64,
) -> Result<Vec<AssetFieldValueView>, CommandError> {
    blocking(&svc, move |u| Ok(u.get_metadata_values(asset_id)?.into_iter().map(Into::into).collect())).await
}

/// 자산 하나의 값을 지정한다. `patches` 에 없는 필드는 그대로, 하나라도 잘못되면 전체 거부.
#[tauri::command]
pub async fn set_asset_metadata(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_id: i64,
    patches: Vec<FieldPatchDto>,
) -> Result<(), CommandError> {
    let patches = to_patches(patches);
    let out = blocking(&svc, move |u| u.set_metadata_values(asset_id, &patches).map(|_| ())).await;
    search.wake();
    out
}

/// 선택한 자산들에 같은 패치를 한 번에 적용한다 (자산 최대 10,000건, 초과 시 `invalid_input`).
/// 실제로 값이 바뀐 자산 수를 돌려준다.
#[tauri::command]
pub async fn bulk_set_asset_metadata(
    svc: Shared<'_>,
    search: Wake<'_>,
    asset_ids: Vec<i64>,
    patches: Vec<FieldPatchDto>,
) -> Result<u64, CommandError> {
    let patches = to_patches(patches);
    let out = blocking(&svc, move |u| u.bulk_set_metadata_values(&asset_ids, &patches)).await;
    search.wake();
    out.map(|n| n as u64)
}

#[cfg(test)]
#[path = "metadata_commands_tests.rs"]
mod metadata_commands_tests;
