//! 파일 메타(크기·수정 시각)와 파일 식별자 (D3).
//!
//! `file_identity` 는 NTFS/ReFS 128-bit 파일 ID + 볼륨 시리얼을 문자열로 만든 값이다.
//! 지원하지 않는 파일시스템(FAT 등)·일부 SMB 는 `None`.

use std::path::Path;
use std::time::SystemTime;

use crate::error::CoreError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub size: u64,
    /// UTC Unix ms (`media_assets.modified_at` 와 같은 단위)
    pub modified_ms: i64,
    pub file_identity: Option<String>,
}

/// 파일 메타·식별자 조회 — OS 호출을 숨겨 reconcile 시험에서 대체할 수 있게 한다.
pub trait FileIdentityProvider {
    fn stat(&self, path: &Path) -> Result<FileMeta, CoreError>;
}

/// `SystemTime` -> Unix ms (1970 이전은 음수, 범위 밖은 포화).
pub fn modified_ms(t: SystemTime) -> i64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |v| -v),
    }
}

/// 파일 ID 없이 크기·수정 시각만 돌려주는 제공자 (파일 ID 를 지원하지 않는 볼륨과 같은 동작).
#[derive(Debug, Default, Clone, Copy)]
pub struct MetadataOnly;

impl FileIdentityProvider for MetadataOnly {
    fn stat(&self, path: &Path) -> Result<FileMeta, CoreError> {
        let (size, modified_ms) = basic_stat(path)?;
        Ok(FileMeta {
            size,
            modified_ms,
            file_identity: None,
        })
    }
}

pub(crate) fn basic_stat(path: &Path) -> Result<(u64, i64), CoreError> {
    let meta = std::fs::metadata(path).map_err(|e| CoreError::io(path, e))?;
    if !meta.is_file() {
        return Err(CoreError::NotAFile(path.to_path_buf()));
    }
    let modified = meta.modified().map_err(|e| CoreError::io(path, e))?;
    Ok((meta.len(), modified_ms(modified)))
}

#[cfg(windows)]
pub use crate::win_identity::WindowsFileIdentity;
