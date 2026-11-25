//! MAMData 저장소 기반 (P2-1) — 위치 결정, 하위 구조, 단일 인스턴스 잠금.
//!
//! 위치는 DB 안에 둘 수 없다(DB 를 찾기 전에 읽어야 함) — 부트스트랩 파일 `location.json` 에 기록한다.
//! 규칙: 환경 변수 `MAM_DATA_DIR`(시험·진단) > `location.json` > 기본 `%LOCALAPPDATA%\MAMDesktop\MAMData`.
//! NAS(UNC·네트워크 드라이브)는 거부한다 (db.md §6).

#[cfg(windows)]
mod instance_mutex;
mod location;
mod lock;
mod overlap;
mod paths;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

#[cfg(windows)]
pub use instance_mutex::InstanceMutex;
pub use location::{
    DriveProbe, LocationSource, ResolvedLocation, SystemDriveProbe, bootstrap_dir, clear_location,
    mirror_file, pending_location, read_mirror, resolve_location, validate_location,
    write_location, write_mirror,
};
pub use lock::InstanceLock;
pub use overlap::{is_same_or_inside, paths_overlap, resolve_data_root};
pub use paths::DataPaths;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// UNC 경로 또는 네트워크 드라이브 — MAMData 는 로컬 드라이브에만 둘 수 있다.
    #[error("network locations are not allowed for MAMData: {}", .0.display())]
    NetworkLocation(PathBuf),
    #[error("MAMData location must be an absolute path: {}", .0.display())]
    NotAbsolute(PathBuf),
    #[error("MAMData location is not writable: {}", .0.display())]
    NotWritable(PathBuf),
    #[error("another instance of velio. is already running")]
    AlreadyRunning,
    #[error("%LOCALAPPDATA% is not set")]
    NoLocalAppData,
}

impl StorageError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
