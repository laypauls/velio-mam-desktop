//! 휴지통 이동 (D3) — 영구 삭제 경로는 없다.
//!
//! [`Trash`] trait 뒤에 숨겨 업무 로직을 가짜 구현으로 시험한다. Windows 구현은 `SHFileOperationW` 에
//! `FOF_ALLOWUNDO` 를 항상 켠다. 휴지통이 없는 드라이브(네트워크·이동식)는 영구 삭제로 이어질 수 있어 거부하고,
//! 용량 초과 등으로 영구 삭제가 될 상황은 `FOF_WANTNUKEWARNING` 으로 확인창을 띄워 사용자가 거부할 수 있게 한다.

use std::path::Path;

use super::ProtectError;

/// 파일 하나를 휴지통으로 보낸다. 성공하면 원래 경로에 파일이 없어야 한다.
pub trait Trash {
    fn trash(&self, path: &Path) -> Result<(), ProtectError>;
}

#[cfg(windows)]
pub use win::RecycleBin;

#[cfg(windows)]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
        FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW, SHFileOperationW,
    };

    use super::Trash;
    use crate::protect::ProtectError;

    /// `GetDriveTypeW` 의 `DRIVE_FIXED`
    const DRIVE_FIXED: u32 = 3;

    /// 실제 Windows 휴지통
    #[derive(Debug, Default, Clone, Copy)]
    pub struct RecycleBin;

    /// 되돌림(undo) 의미가 빠지지 않게 플래그를 한 곳에서 조립한다
    pub(super) fn delete_flags() -> u16 {
        let flags =
            FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING;
        flags as u16
    }

    impl Trash for RecycleBin {
        fn trash(&self, path: &Path) -> Result<(), ProtectError> {
            let text = crate::volume::strip_verbatim(&path.to_string_lossy()).replace('/', "\\");
            let bytes = text.as_bytes();
            let is_drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'\\';
            if !is_drive {
                tracing::warn!(path = %text, "recycle bin needs an absolute local drive path");
                return Err(ProtectError::InvalidInput("invalid_input".into()));
            }
            if text.contains(['*', '?']) {
                return Err(ProtectError::InvalidInput(
                    "wildcards are not allowed".into(),
                ));
            }
            if !path.is_file() {
                return Err(ProtectError::Offline("file_missing".into()));
            }
            let root: Vec<u16> = text[..3].encode_utf16().chain([0]).collect();
            // SAFETY: root 는 NUL 로 끝나는 유효한 UTF-16 문자열이다.
            if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
                tracing::warn!(
                    drive = &text[..2],
                    "drive has no recycle bin; refusing to delete"
                );
                return Err(ProtectError::Io("no_recycle_bin".into()));
            }
            // pFrom: 이중 NUL 종료 목록
            let from: Vec<u16> = std::ffi::OsStr::new(&text)
                .encode_wide()
                .chain([0, 0])
                .collect();
            let mut op = SHFILEOPSTRUCTW {
                hwnd: std::ptr::null_mut(),
                wFunc: FO_DELETE,
                pFrom: from.as_ptr(),
                pTo: std::ptr::null(),
                fFlags: delete_flags(),
                fAnyOperationsAborted: 0,
                hNameMappings: std::ptr::null_mut(),
                lpszProgressTitle: std::ptr::null(),
            };
            // SAFETY: op 의 포인터는 호출 동안 살아 있는 버퍼를 가리키고 pFrom 은 이중 NUL 로 끝난다.
            let rc = unsafe { SHFileOperationW(&raw mut op) };
            let aborted = op.fAnyOperationsAborted != 0;
            if rc != 0 {
                tracing::warn!(code = rc, path = %text, "recycle bin operation failed");
                return Err(ProtectError::Io("recycle_failed".into()));
            }
            if aborted || path.exists() {
                tracing::warn!(path = %text, "file was not moved to the recycle bin");
                return Err(ProtectError::Io("recycle_failed".into()));
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        //! 분류: Business rule — 영구 삭제 방지 플래그
        use super::*;

        #[test]
        fn t_prot_trash_01_flags_always_include_allow_undo() {
            let f = u32::from(delete_flags());
            assert_ne!(f & FOF_ALLOWUNDO, 0);
            assert_ne!(f & FOF_WANTNUKEWARNING, 0);
        }
    }
}
