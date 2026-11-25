//! 재활용 command (P2-8): 내보내기, 파일 클립보드, 원본 열기, 탐색기에서 위치 열기.
//! 로직은 `mam_core::reuse::Reuse` 에 있다. 내보내기는 작업을 시작만 하고 `export://progress` 이벤트로 알린다.
//! 계약 정본: UI 계약 `ipc-contract.ts` (P2-8), TS 쪽 `src/lib/ipc.ts`.

use std::sync::Arc;

use mam_core::reuse::{ExportInput, ProgressSink, Reuse, TaskStarted, WindowsClipboard};
use mam_core::scan::IngestError;
use tauri::{Emitter, State};

use crate::error::{CommandError, ErrorCode};
use crate::ingest_commands::run_blocking;

/// 계약 `EXPORT_PROGRESS_EVENT`
pub const EXPORT_PROGRESS_EVENT: &str = "export://progress";

type Shared<'a> = State<'a, Arc<Reuse>>;

async fn blocking<T, F>(svc: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Reuse) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

/// 내보내기 시작. 검증에 실패하면 오류로 reject 하고, 통과하면 `taskId` 를 돌려준 뒤 백그라운드에서 복사한다.
/// 이벤트는 응답보다 먼저 도착할 수 있다 (프론트가 `taskId` 별로 모은다).
#[tauri::command]
pub async fn export_assets(
    app: tauri::AppHandle,
    reuse: Shared<'_>,
    input: ExportInput,
) -> Result<TaskStarted, CommandError> {
    let sink: ProgressSink = Arc::new(move |p| {
        if let Err(e) = app.emit(EXPORT_PROGRESS_EVENT, &p) {
            tracing::warn!(%e, "cannot emit export progress");
        }
    });
    blocking(&reuse, move |r| r.start_export(input, sink)).await
}

/// 원본 경로 목록을 Windows 파일 클립보드(CF_HDROP)에 넣는다. 오프라인·파일 없음 자산은 제외한다.
#[tauri::command]
pub async fn copy_to_clipboard(
    window: tauri::WebviewWindow,
    reuse: Shared<'_>,
    asset_ids: Vec<i64>,
) -> Result<(), CommandError> {
    let hwnd = window.hwnd().map_err(|e| {
        CommandError::logged(
            ErrorCode::Internal,
            "clipboard owner window is unavailable",
            &e,
        )
    })?;
    let clipboard = WindowsClipboard {
        hwnd: hwnd.0 as isize,
    };
    blocking(&reuse, move |r| {
        r.copy_to_clipboard(&asset_ids, &clipboard).map(|_| ())
    })
    .await
}

#[tauri::command]
pub async fn open_original(reuse: Shared<'_>, asset_id: i64) -> Result<(), CommandError> {
    blocking(&reuse, move |r| r.open_original(asset_id)).await
}

#[tauri::command]
pub async fn reveal_in_explorer(reuse: Shared<'_>, asset_id: i64) -> Result<(), CommandError> {
    blocking(&reuse, move |r| r.reveal_in_explorer(asset_id)).await
}

#[cfg(test)]
mod tests {
    // Categories: Validation (계약 이벤트 이름) / Business rule (서비스 오류 -> 계약 코드, 경로 비노출)
    use super::*;

    #[test]
    fn t_ipc_19_export_event_name_matches_the_frontend_contract() {
        assert_eq!(EXPORT_PROGRESS_EVENT, "export://progress");
    }

    #[test]
    fn t_ipc_20_reuse_and_user_data_errors_reach_the_caller_as_contract_codes() {
        // Arrange: 서비스가 돌려주는 오류 (오프라인·없음·중복·잘못된 입력)
        let cases: Vec<(IngestError, ErrorCode)> = vec![
            (IngestError::Offline, ErrorCode::Offline),
            (IngestError::NotFound, ErrorCode::NotFound),
            (IngestError::AlreadyRegistered, ErrorCode::AlreadyRegistered),
            (
                IngestError::InvalidInput("destDir"),
                ErrorCode::InvalidInput,
            ),
            (
                IngestError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)),
                ErrorCode::DiskFull,
            ),
        ];

        for (err, code) in cases {
            // Act: command 가 쓰는 변환 경로(run_blocking)를 그대로 통과시킨다
            let result: Result<(), CommandError> =
                tauri::async_runtime::block_on(run_blocking(Arc::new(()), move |_| Err(err)));

            // Assert
            let mapped = result.unwrap_err();
            assert_eq!(mapped.code(), code);
            let json = serde_json::to_value(&mapped).unwrap();
            assert_eq!(json["code"], serde_json::to_value(code).unwrap());
            assert!(json["message"].is_string());
        }
    }
}
