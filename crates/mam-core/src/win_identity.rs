//! Windows 파일 ID 구현 — 원본은 메타데이터 읽기 권한(`FILE_READ_ATTRIBUTES`)으로만 연다.

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandleEx,
};

use crate::error::CoreError;
use crate::identity::{FileIdentityProvider, FileMeta, basic_stat};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsFileIdentity;

impl FileIdentityProvider for WindowsFileIdentity {
    fn stat(&self, path: &Path) -> Result<FileMeta, CoreError> {
        let (size, modified_ms) = basic_stat(path)?;
        Ok(FileMeta {
            size,
            modified_ms,
            file_identity: file_identity(path)?,
        })
    }
}

/// `<볼륨 시리얼 16hex>:<파일 ID 32hex>`. 파일시스템이 FileIdInfo 를 지원하지 않으면 `None`.
fn file_identity(path: &Path) -> Result<Option<String>, CoreError> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|e| CoreError::io(path, e))?;
    // SAFETY: FILE_ID_INFO 는 모든 비트가 유효한 POD 이고, 핸들은 `file` 이 살아 있는 동안 유효하다.
    let (ok, info) = unsafe {
        let mut info: FILE_ID_INFO = std::mem::zeroed();
        let ok = GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        );
        (ok != 0, info)
    };
    if !ok {
        return Ok(None);
    }
    let id: String = info
        .FileId
        .Identifier
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some(format!("{:016x}:{id}", info.VolumeSerialNumber)))
}
