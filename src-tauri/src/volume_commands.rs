//! 새 디스크 command (계약: `volumes://pending-changed` 이벤트 + 4개 command).
//! 판정·상태는 `mam_core::ingest::Ingest` 에 있고 여기서는 `spawn_blocking`·오류 변환·이벤트 연결만 한다.

use std::sync::Arc;

use mam_core::ingest::{
    EjectPrepared, FolderRemovalProgress, Ingest, KnownVolume, PENDING_CHANGED_EVENT, PendingChangedPayload, PendingVolume,
    REMOVAL_PROGRESS_EVENT,
};
use mam_db::ignored_volumes::IgnoredVolume;
use tauri::State;

use crate::ai::EventSink;
use crate::error::CommandError;

type Shared<'a> = State<'a, Arc<Ingest>>;

/// 대기 목록이 바뀔 때마다 `volumes://pending-changed { volumes }` 를 내보내도록 연결한다. `Ingest::start` 전에 호출한다.
pub fn wire_pending_events(ingest: &Ingest, sink: Arc<dyn EventSink>) {
    ingest.set_pending_listener(Arc::new(move |volumes| {
        match serde_json::to_value(PendingChangedPayload { volumes }) {
            Ok(v) => sink.emit(PENDING_CHANGED_EVENT, v),
            Err(e) => tracing::debug!(%e, "cannot serialize pending volumes event"),
        }
    }));
}

/// 폴더 제거 진행(`folders://removal-progress { folderId, done, total, finished, error }`)을 이벤트로 내보내도록 연결한다.
/// `Ingest::start` 전에 호출한다 (시작 때 이어서 하는 제거도 알려야 하므로).
pub fn wire_removal_events(ingest: &Ingest, sink: Arc<dyn EventSink>) {
    ingest.set_removal_listener(Arc::new(move |p: FolderRemovalProgress| {
        match serde_json::to_value(p) {
            Ok(v) => sink.emit(REMOVAL_PROGRESS_EVENT, v),
            Err(e) => tracing::debug!(%e, "cannot serialize removal progress event"),
        }
    }));
}

/// 앱 초기 로드 때 조회하는 현재 대기 목록.
#[tauri::command]
pub async fn list_pending_new_volumes(ingest: Shared<'_>) -> Result<Vec<PendingVolume>, CommandError> {
    Ok(ingest.pending_volumes())
}

/// "나중에". `remember` 면 묻지 않는 디스크로 저장한다. 모르는 키는 아무 일도 없이 성공.
#[tauri::command]
pub async fn dismiss_new_volume(
    ingest: Shared<'_>,
    volume_key: String,
    remember: bool,
) -> Result<(), CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.dismiss_new_volume(&volume_key, remember)).await
}

/// 설정 "묻지 않는 디스크" 목록.
#[tauri::command]
pub async fn list_ignored_volumes(ingest: Shared<'_>) -> Result<Vec<IgnoredVolume>, CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.list_ignored_volumes()).await
}

/// [다시 묻기]. 지금 연결된 디스크면 바로 대기 목록에 오르고 이벤트가 나간다.
#[tauri::command]
pub async fn unignore_volume(ingest: Shared<'_>, volume_key: String) -> Result<(), CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.unignore_volume(&volume_key)).await
}

/// 설정 "기억하는 디스크": `volumes` 행 + 묻지 않음 항목을 `volumeKey` 당 1행으로 합친 목록.
#[tauri::command]
pub async fn list_known_volumes(ingest: Shared<'_>) -> Result<Vec<KnownVolume>, CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.list_known_volumes()).await
}

/// 디스크 기록 지우기 (`volumes` 행 + 묻지 않음 항목). 관리 폴더(제거 진행 중 포함)가 있으면 `volume_has_folders`,
/// 정리되지 않은 옛 보관 자산을 지워야 하는데 사전 백업이 실패하면 `backup_failed`. 모르는 키는 아무 일 없이 성공.
/// 원본 파일과 라이브러리 정보는 바뀌지 않는다.
#[tauri::command]
pub async fn forget_volume_record(ingest: Shared<'_>, volume_key: String) -> Result<(), CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.forget_volume_record(&volume_key)).await
}

/// 폴더가 있는 USB 디스크를 안전하게 꺼낼 수 있게 앱이 손을 뗀다 (감시 중지·실행 중 스캔/처리 취소(대기열 유지)·FFmpeg 종료·재생 정지·
/// `availability = offline`) - 모두 끝낸 뒤 반환한다. 네트워크 폴더는 `invalid_input`, 이미 오프라인이면 `offline`,
/// 이미 준비 상태면 같은 결과로 성공(멱등), 제한 시간 안에 파일을 놓지 않으면 `busy`.
#[tauri::command]
pub async fn prepare_volume_eject(ingest: Shared<'_>, folder_id: i64) -> Result<EjectPrepared, CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.prepare_volume_eject(folder_id)).await
}

/// 꺼내기 준비를 풀고 기존 재연결 처리(온라인·감시·따라잡기 재스캔)를 한다. 준비 상태가 아니면 아무 일 없이 성공.
#[tauri::command]
pub async fn resume_volume_watch(ingest: Shared<'_>, folder_id: i64) -> Result<(), CommandError> {
    let ingest = Arc::clone(&ingest);
    run(move || ingest.resume_volume_watch(folder_id)).await
}

async fn run<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, mam_core::scan::IngestError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

#[cfg(test)]
#[path = "volume_commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "volume_eject_tests.rs"]
pub(crate) mod eject_tests;
