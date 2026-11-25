//! 재생 구간 command: 자산별 구간 조회·전체 교체, 컬렉션 구간 요약. 로직은 `mam_core::userdata::UserData`
//! (`play_ranges.rs`)에 있고 여기서는 인자 변환·`spawn_blocking`·오류 변환만 한다. 구간은 검색 색인 대상이 아니라
//! `search.wake()` 를 호출하지 않는다. 오류: `not_found`(없는 자산·컬렉션, 활성 아님), `invalid_input`(이미지·잘못된 구간).

use std::sync::Arc;

use mam_core::scan::IngestError;
use mam_core::userdata::{PlayRange, PlayRangeSummary, UserData};
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

/// 자산의 재생 구간 (start 오름차순, 없으면 `[]`).
#[tauri::command]
pub async fn play_ranges_get(
    svc: Shared<'_>,
    asset_id: i64,
) -> Result<Vec<PlayRange>, CommandError> {
    blocking(&svc, move |u| u.play_ranges_get(asset_id)).await
}

/// 자산의 재생 구간을 정규화해 전체 교체하고 저장된 결과를 돌려준다.
#[tauri::command]
pub async fn play_ranges_set(
    svc: Shared<'_>,
    asset_id: i64,
    ranges: Vec<PlayRange>,
) -> Result<Vec<PlayRange>, CommandError> {
    blocking(&svc, move |u| u.play_ranges_set(asset_id, &ranges)).await
}

/// 컬렉션 항목 중 구간이 있는 자산만의 요약 (희소).
#[tauri::command]
pub async fn collection_play_range_summary(
    svc: Shared<'_>,
    collection_id: i64,
) -> Result<Vec<PlayRangeSummary>, CommandError> {
    blocking(&svc, move |u| u.collection_play_range_summary(collection_id)).await
}

#[cfg(test)]
#[path = "play_range_commands_tests.rs"]
mod play_range_commands_tests;
