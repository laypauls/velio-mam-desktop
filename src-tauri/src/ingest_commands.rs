//! 수집 command (P2-2, D8): 관리 폴더, 작업 요약·일시정지, 스캔 오류, 폴더 선택 대화상자.
//! 로직은 `mam_core::ingest::Ingest` 에 있고 여기서는 인자 검증·오류 변환·`spawn_blocking` 만 한다.
//! Rust <-> TS 계약: `src/lib/ipc.ts` (P2-2 절), 정본은 UI 계약 `ipc-contract.ts`.

use std::sync::Arc;

use mam_core::ingest::{Ingest, JobSummary, RelocateOutcome};
use mam_core::jobs::JobKind;
use mam_core::purge::FolderRemovalImpact;
use mam_core::scan::{
    FolderNode, IngestError, ManagedFolderView, MediaTypeCounts, MediaTypeSet, ScanErrorView,
};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use crate::error::{CommandError, ErrorCode};

impl From<IngestError> for CommandError {
    fn from(e: IngestError) -> Self {
        match e {
            // 정적 문구만 사용자에게 간다 (내부 경로·SQL 은 로그에만)
            IngestError::InvalidInput(what) => {
                Self::new(ErrorCode::InvalidInput, format!("invalid input: {what}"))
            }
            IngestError::NotFound => Self::new(ErrorCode::NotFound, "not found"),
            IngestError::AlreadyRegistered => {
                Self::new(ErrorCode::AlreadyRegistered, "already registered")
            }
            // 이전하려는 경로가 현재 경로와 같다 — 별도 코드 없이 "이미 등록됨" 으로 합친다 (경로는 싣지 않는다)
            IngestError::SamePath => {
                Self::new(ErrorCode::AlreadyRegistered, "folder is already at this path")
            }
            IngestError::ScanRunning => {
                Self::new(ErrorCode::ScanRunning, "a scan of this folder is running")
            }
            IngestError::NestedFolder {
                inside_existing: true,
            } => Self::new(
                ErrorCode::NestedFolder,
                "folder is already included in a registered folder",
            ),
            IngestError::NestedFolder {
                inside_existing: false,
            } => Self::new(
                ErrorCode::NestedFolder,
                "folder contains an already registered folder",
            ),
            IngestError::OverlapsManagedData { contains } => Self::new(
                ErrorCode::NestedFolder,
                if contains {
                    "folder contains the MAM data or backup directory"
                } else {
                    "folder is inside the MAM data or backup directory"
                },
            ),
            IngestError::Offline => Self::new(ErrorCode::Offline, "volume is offline"),
            IngestError::Busy => Self::new(ErrorCode::Busy, "a folder in this range is being removed"),
            IngestError::BackupFailed => {
                Self::new(ErrorCode::BackupFailed, "database backup before removal failed")
            }
            IngestError::VolumeHasFolders => {
                Self::new(ErrorCode::VolumeHasFolders, "volume still has managed folders")
            }
            IngestError::Io(ref io) if io.kind() == std::io::ErrorKind::StorageFull => {
                Self::logged(ErrorCode::DiskFull, "not enough disk space", &e)
            }
            IngestError::Io(ref io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
                Self::logged(ErrorCode::Io, "access denied", &e)
            }
            IngestError::Io(_) | IngestError::Core(_) => {
                Self::logged(ErrorCode::Io, "file operation failed", &e)
            }
            IngestError::Pool(p) => p.into(),
            IngestError::Db(d) => d.into(),
            IngestError::Jobs(_) | IngestError::Sql(_) => {
                Self::logged(ErrorCode::Internal, "database operation failed", &e)
            }
        }
    }
}

/// 계약 `Page<T>`
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub next_cursor: Option<String>,
}

/// 서비스 메서드를 `spawn_blocking` 에서 실행하고 오류를 계약 `CommandError` 로 바꾼다 (사용자 데이터·재활용 command 공용).
pub(crate) async fn run_blocking<S, T, F>(svc: Arc<S>, f: F) -> Result<T, CommandError>
where
    S: Send + Sync + 'static,
    T: Send + 'static,
    F: FnOnce(&S) -> Result<T, IngestError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || f(&svc))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

type Shared<'a> = State<'a, Arc<Ingest>>;

async fn blocking<T, F>(ingest: &Shared<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&Ingest) -> Result<T, IngestError> + Send + 'static,
{
    let ingest = Arc::clone(ingest);
    tauri::async_runtime::spawn_blocking(move || f(&ingest))
        .await
        .map_err(|e| {
            tracing::error!(%e, "blocking task failed");
            CommandError::internal()
        })?
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_folders(ingest: Shared<'_>) -> Result<Vec<ManagedFolderView>, CommandError> {
    blocking(&ingest, |i| i.list_folders()).await
}

/// 관리 폴더 추가. `media_types` 가 모두 false 면 `invalid_input`.
#[tauri::command]
pub async fn add_folder(
    ingest: Shared<'_>,
    path: String,
    media_types: MediaTypeSet,
) -> Result<ManagedFolderView, CommandError> {
    blocking(&ingest, move |i| i.add_folder_with_types(&path, media_types)).await
}

/// 폴더를 새 경로로 옮겨 연결한다 (자산·태그·메모 유지). 새 경로 스캔이 예약된다.
#[tauri::command]
pub async fn relocate_folder(
    ingest: Shared<'_>,
    folder_id: i64,
    new_path: String,
) -> Result<RelocateOutcome, CommandError> {
    blocking(&ingest, move |i| i.relocate_folder(folder_id, &new_path)).await
}

/// 폴더 제거 확인 Dialog 용 — 제거하면 지워지는 항목 수. 없는 폴더·제거 중인 폴더는 `not_found`.
#[tauri::command]
pub async fn folder_removal_impact(
    ingest: Shared<'_>,
    folder_id: i64,
) -> Result<FolderRemovalImpact, CommandError> {
    blocking(&ingest, move |i| i.folder_removal_impact(folder_id)).await
}

/// 관리 폴더 제거 — 사전 DB 백업(실패하면 `backup_failed`, 아무것도 바꾸지 않음) 후 폴더를 제거 중으로 바꿔 목록·검색에서
/// 즉시 뺀다. 연결 데이터(태그·메모·컬렉션 소속·AI 결과·썸네일 등) 삭제는 백그라운드로 이어지며(`folders://removal-progress`)
/// 앱을 다시 시작해도 이어진다. 원본 파일은 건드리지 않는다.
#[tauri::command]
pub async fn remove_folder(ingest: Shared<'_>, folder_id: i64) -> Result<(), CommandError> {
    blocking(&ingest, move |i| i.remove_folder(folder_id)).await
}

#[tauri::command]
pub async fn rescan_folder(ingest: Shared<'_>, folder_id: i64) -> Result<(), CommandError> {
    blocking(&ingest, move |i| i.rescan_folder(folder_id)).await
}

/// 폴더 숨김 토글 (보기 속성 - 스캔·분석은 계속). 없는 폴더·등록 해제 중이면 `NotFound`, 같은 값 재설정은 성공.
#[tauri::command]
pub async fn set_folder_hidden(
    ingest: Shared<'_>,
    folder_id: i64,
    hidden: bool,
) -> Result<(), CommandError> {
    blocking(&ingest, move |i| i.set_folder_hidden(folder_id, hidden)).await
}

/// 폴더 별칭 설정/해제 (표시 전용). 빈 값은 해제, 64자 초과는 `InvalidInput`, 없는 폴더는 `NotFound`.
#[tauri::command]
pub async fn set_folder_alias(
    ingest: Shared<'_>,
    folder_id: i64,
    alias: Option<String>,
) -> Result<(), CommandError> {
    blocking(&ingest, move |i| i.set_folder_alias(folder_id, alias)).await
}

/// 제외되는 유형의 수집 항목을 어떻게 할지 (계약 `mode`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaTypeChangeMode {
    /// 수집된 항목 유지 — 새 파일만 추가하지 않는다
    Keep,
    /// 제외된 유형 자산의 연결 데이터(태그·메모·컬렉션 소속·AI 결과·썸네일 등)를 영구 삭제 (원본 무변경, 사전 DB 백업)
    Purge,
}

/// 계약 `{ detached }` — `purge` 로 연결 데이터를 지운 위치 수
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SetMediaTypesResult {
    pub detached: u64,
}

/// 스캔 대상 유형 변경 + 재스캔 예약. 모두 false 면 `invalid_input`, 없는 폴더는 `not_found`, `keep` 이면 `detached` = 0,
/// `purge` 의 사전 백업이 실패하면 `backup_failed` (유형도 바뀌지 않는다).
#[tauri::command]
pub async fn set_folder_media_types(
    ingest: Shared<'_>,
    folder_id: i64,
    media_types: MediaTypeSet,
    mode: MediaTypeChangeMode,
) -> Result<SetMediaTypesResult, CommandError> {
    let purge = mode == MediaTypeChangeMode::Purge;
    let detached = blocking(&ingest, move |i| {
        i.set_folder_media_types(folder_id, media_types, purge)
    })
    .await?;
    Ok(SetMediaTypesResult { detached })
}

/// 폴더 하위 활성 위치의 유형별 개수. 없는 폴더는 `not_found`.
#[tauri::command]
pub async fn count_folder_assets_by_type(
    ingest: Shared<'_>,
    folder_id: i64,
) -> Result<MediaTypeCounts, CommandError> {
    blocking(&ingest, move |i| i.count_folder_assets_by_type(folder_id)).await
}

#[tauri::command]
pub async fn list_subfolders(
    ingest: Shared<'_>,
    folder_id: i64,
    rel_path: Option<String>,
) -> Result<Vec<FolderNode>, CommandError> {
    blocking(&ingest, move |i| {
        i.list_subfolders(folder_id, rel_path.as_deref())
    })
    .await
}

#[tauri::command]
pub async fn job_summary(ingest: Shared<'_>) -> Result<JobSummary, CommandError> {
    blocking(&ingest, |i| i.summary()).await
}

#[tauri::command]
pub async fn pause_jobs(
    ingest: Shared<'_>,
    kind: Option<JobKind>,
) -> Result<JobSummary, CommandError> {
    blocking(&ingest, move |i| i.pause_jobs(kind)).await
}

#[tauri::command]
pub async fn resume_jobs(
    ingest: Shared<'_>,
    kind: Option<JobKind>,
) -> Result<JobSummary, CommandError> {
    blocking(&ingest, move |i| i.resume_jobs(kind)).await
}

#[tauri::command]
pub async fn list_scan_errors(
    ingest: Shared<'_>,
    cursor: Option<String>,
    limit: u32,
) -> Result<Page<ScanErrorView>, CommandError> {
    blocking(&ingest, move |i| {
        let (items, total, next_cursor) = i.list_scan_errors(
            cursor.as_deref(),
            usize::try_from(limit).unwrap_or(usize::MAX),
        )?;
        Ok(Page {
            items,
            total,
            next_cursor,
        })
    })
    .await
}

#[tauri::command]
pub async fn retry_scan_errors(
    ingest: Shared<'_>,
    error_ids: Option<Vec<i64>>,
) -> Result<u32, CommandError> {
    blocking(&ingest, move |i| {
        let n = i.retry_scan_errors(error_ids.as_deref())?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    })
    .await
}

/// 선택 결과 -> 절대 경로 문자열 (취소는 `None`). 경로가 아닌 선택(URI 등)은 거부한다.
pub fn picked_to_text(
    picked: Option<tauri_plugin_dialog::FilePath>,
) -> Result<Option<String>, CommandError> {
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|_| CommandError::new(ErrorCode::InvalidInput, "selection is not a file path"))?;
    if !path.is_absolute() {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "selection is not an absolute path",
        ));
    }
    Ok(Some(mam_core::volume::strip_verbatim(
        &path.to_string_lossy(),
    )))
}

/// 대화상자 시작 위치: 값이 있고 실제로 존재하는 디렉터리일 때만 (없으면 기존 동작 그대로).
fn existing_dir(path: Option<&str>) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(path?);
    (p.is_absolute() && p.is_dir()).then(|| p.to_path_buf())
}

/// 폴더 선택 대화상자 (폴더 등록·내보내기 대상·백업 폴더 선택용). 취소하면 `null`.
/// 대화상자는 자체 command 로만 열고 JS 대화상자 플러그인 권한은 열지 않는다.
#[tauri::command]
pub async fn pick_directory(
    app: tauri::AppHandle,
    default_path: Option<String>,
) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = std::sync::mpsc::channel();
    let mut dialog = app.dialog().file();
    if let Some(dir) = existing_dir(default_path.as_deref()) {
        dialog = dialog.set_directory(dir);
    }
    if let Some(window) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| {
            tracing::error!(%e, "folder dialog task failed");
            CommandError::internal()
        })?
        .map_err(|e| {
            tracing::error!(%e, "folder dialog closed without a result");
            CommandError::internal()
        })?;
    picked_to_text(picked)
}

#[cfg(test)]
#[path = "ingest_commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ingest_relocate_tests.rs"]
mod relocate_tests;

#[cfg(test)]
#[path = "purge_commands_tests.rs"]
mod purge_tests;
