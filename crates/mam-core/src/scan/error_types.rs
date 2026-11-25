use mam_db::{DbError, PoolError};

use crate::CoreError;
use crate::jobs::JobsError;

/// 수집 계층 오류. command 경계에서 사용자 메시지 키(`ErrorCode`)로 변환한다.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("not found")]
    NotFound,
    #[error("folder is already registered")]
    AlreadyRegistered,
    /// 이전하려는 경로가 현재 경로와 같다 (명령 경계에서는 `already_registered` 로 합친다)
    #[error("folder is already at this path")]
    SamePath,
    /// 이 폴더의 스캔 작업이 실행 중이라 위치를 옮길 수 없다
    #[error("a scan of this folder is running")]
    ScanRunning,
    /// `inside_existing` = 이미 등록된 상위 폴더에 포함됨 / false = 이미 등록된 하위 폴더를 포함함
    #[error("nested folder (inside existing: {inside_existing})")]
    NestedFolder { inside_existing: bool },
    /// 폴더가 MAMData·백업 폴더(MAM 산출물 위치)와 겹침. `contains` = 폴더가 그 위치를 포함함 / false = 그 안에 있음
    #[error("folder overlaps the MAM data or backup directory (contains: {contains})")]
    OverlapsManagedData { contains: bool },
    #[error("volume is offline")]
    Offline,
    /// 같은 범위의 폴더가 제거 중이라 지금은 할 수 없다 (추가·경로 변경·재스캔)
    #[error("a folder in this range is being removed")]
    Busy,
    /// 제거 직전 데이터베이스 백업에 실패했다 - 아무것도 바꾸지 않았다
    #[error("database backup before removal failed")]
    BackupFailed,
    /// 디스크에 관리 폴더(제거 진행 중 포함)가 남아 있어 기록을 지울 수 없다
    #[error("volume still has managed folders")]
    VolumeHasFolders,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Pool(#[from] PoolError),
    #[error(transparent)]
    Jobs(#[from] JobsError),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
}
