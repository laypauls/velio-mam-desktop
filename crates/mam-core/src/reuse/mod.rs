//! 재활용 (P2-8) — 내보내기(복사), 파일 클립보드, 원본 열기, 탐색기에서 위치 열기.
//!
//! 원본은 **읽기만** 한다: 내보내기는 원본을 읽어 대상 폴더에 새 파일을 쓰고, 클립보드는 원본 경로 목록만 넣고,
//! 열기는 OS 기본 프로그램에 맡긴다. OS 부수효과(창 열기·클립보드·파일 복사)는 trait 뒤에 두어 시험에서 갈아끼운다.

mod clipboard;
mod export;
mod fsops;
mod manifest;
mod resolve;
#[cfg(windows)]
mod win;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use mam_db::DbPool;
use serde::{Deserialize, Serialize};

use crate::SharedClock;
use crate::scan::IngestError;

pub use clipboard::build_drop_files;
pub use export::{AbortReason, ExportJob, ExportReport};
pub use fsops::StdFs;
pub use resolve::{Access, RESOLVE_SQL};
#[cfg(windows)]
pub use win::{ShellLauncher, WindowsClipboard};

/// 한 번에 다루는 자산 수 상한 (사용자 데이터와 같은 값)
pub use crate::userdata::MAX_ASSETS_PER_CALL;

/// 같은 이름 충돌 정책 (계약 `ConflictPolicy`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    Skip,
    Rename,
    Overwrite,
}

/// `export_assets` 인자 (계약 `ExportInput`)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInput {
    pub asset_ids: Vec<i64>,
    pub dest_dir: String,
    pub conflict: ConflictPolicy,
    pub include_manifest: bool,
}

/// `export_assets` 반환 (계약 `TaskStarted`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStarted {
    pub task_id: String,
}

/// `export://progress` 페이로드 (계약 `ExportProgress`). `done` = 처리한 건수(성공 + 실패 + 건너뜀).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub task_id: String,
    pub done: u64,
    pub total: u64,
    pub failed: u64,
    pub skipped: u64,
    pub finished: bool,
}

/// 진행 이벤트 수신자 (Tauri 쪽은 `emit`, 시험은 채널)
pub type ProgressSink = Arc<dyn Fn(ExportProgress) + Send + Sync>;

/// 내보내기가 쓰는 파일 연산. 시험에서 디스크 가득 참 같은 실패를 주입한다.
pub trait ExportFs: Send + Sync {
    /// `src` 를 읽어 `dst` 에 쓴다. `replace = false` 면 이미 있을 때 `AlreadyExists`, `true` 면 덮어쓴다.
    /// 실패하면 만든 부분 파일은 지운다. `src` 는 읽기 전용으로만 연다.
    fn copy(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()>;
    fn write(&self, dst: &Path, data: &[u8], replace: bool) -> io::Result<()>;
}

/// OS 기본 프로그램으로 열기 / 탐색기에서 선택. 시험에서는 호출만 기록한다.
pub trait Launcher: Send + Sync {
    fn open(&self, path: &Path) -> io::Result<()>;
    fn reveal(&self, path: &Path) -> io::Result<()>;
}

/// 파일 목록 클립보드(CF_HDROP). 창 핸들이 필요해 호출마다 만든다.
pub trait Clipboard {
    fn set_files(&self, paths: &[PathBuf]) -> io::Result<()>;
}

pub struct Reuse {
    pub(crate) pool: Arc<DbPool>,
    pub(crate) clock: SharedClock,
    pub(crate) fs: Arc<dyn ExportFs>,
    pub(crate) launcher: Arc<dyn Launcher>,
    /// 내보내기 대상으로 금지할 추가 경로 (MAM 데이터 디렉터리·백업 폴더). 관리 폴더는 DB 에서 읽는다.
    pub(crate) protected: Vec<PathBuf>,
    pub(crate) seq: AtomicU64,
}

impl Reuse {
    pub fn new(
        pool: Arc<DbPool>,
        clock: SharedClock,
        fs: Arc<dyn ExportFs>,
        launcher: Arc<dyn Launcher>,
        protected: Vec<PathBuf>,
    ) -> Self {
        Self {
            pool,
            clock,
            fs,
            launcher,
            protected,
            seq: AtomicU64::new(0),
        }
    }

    /// 기본 연결 프로그램으로 원본을 연다. 오프라인이면 `Offline`, 파일이 없으면 `NotFound`.
    pub fn open_original(&self, asset_id: i64) -> Result<(), IngestError> {
        let path = self.resolve_existing(asset_id, true)?;
        if resolve::is_executable_name(&path) {
            return Err(IngestError::InvalidInput("file type cannot be opened"));
        }
        self.launcher.open(&path).map_err(IngestError::from)
    }

    /// 원본 파일의 현재 경로 (읽기 전용 재생용). 오프라인이면 `Offline`, 자산·파일이 없으면 `NotFound`.
    pub fn original_path(&self, asset_id: i64) -> Result<PathBuf, IngestError> {
        self.resolve_existing(asset_id, true)
    }

    /// 탐색기에서 원본 위치를 연다(파일 선택). 오프라인이면 `Offline`, 파일이 없으면 `NotFound`.
    pub fn reveal_in_explorer(&self, asset_id: i64) -> Result<(), IngestError> {
        let path = self.resolve_existing(asset_id, false)?;
        self.launcher.reveal(&path).map_err(IngestError::from)
    }

    /// 선택한 자산의 원본 경로를 파일 클립보드에 넣는다. 오프라인·파일 없음 자산은 제외하고 넣은 개수를 돌려준다.
    /// 하나도 넣을 수 없으면 오프라인이 있었으면 `Offline`, 아니면 `NotFound`.
    pub fn copy_to_clipboard(
        &self,
        asset_ids: &[i64],
        clipboard: &dyn Clipboard,
    ) -> Result<usize, IngestError> {
        let ids = crate::userdata::clean_ids(asset_ids)?;
        if ids.is_empty() {
            return Err(IngestError::InvalidInput("assetIds"));
        }
        let rows = self.pool.read(|c| Ok(resolve::resolve(c, &ids)))??;
        let mut paths = Vec::new();
        let mut saw_offline = false;
        for row in rows {
            match row.access {
                Access::Ready(p) if p.is_file() => paths.push(p),
                Access::Offline => saw_offline = true,
                _ => {}
            }
        }
        if paths.is_empty() {
            return Err(if saw_offline {
                IngestError::Offline
            } else {
                IngestError::NotFound
            });
        }
        clipboard.set_files(&paths)?;
        Ok(paths.len())
    }

    /// 자산 1건의 절대 경로. 디스크에 실제로 있는 파일(`file_only`) 또는 항목이어야 한다.
    fn resolve_existing(&self, asset_id: i64, file_only: bool) -> Result<PathBuf, IngestError> {
        let rows = self.pool.read(|c| Ok(resolve::resolve(c, &[asset_id])))??;
        let Some(row) = rows.into_iter().next() else {
            return Err(IngestError::NotFound);
        };
        match row.access {
            Access::Ready(p) if (file_only && p.is_file()) || (!file_only && p.exists()) => Ok(p),
            Access::Offline => Err(IngestError::Offline),
            _ => Err(IngestError::NotFound),
        }
    }
}
