//! 덮어쓰지 않는 파일 이동. `std::fs::rename` 은 Windows 에서 기존 대상을 조용히 덮어쓰므로
//! (존재 확인과 rename 사이에 만들어진 파일이 사라진다) 원본·사용자 경로의 이동에는 이 함수를 쓴다.

use std::io;
use std::path::Path;

/// `src` 를 `dst` 로 옮긴다. `dst` 가 이미 있으면 `ErrorKind::AlreadyExists` 이고 둘 다 그대로다.
/// 같은 볼륨에서만 동작한다 (다른 볼륨이면 오류). `write_through` 는 디스크 기록 완료까지 기다린다.
#[cfg(windows)]
pub(super) fn move_no_replace(src: &Path, dst: &Path, write_through: bool) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};

    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() };
    let (s, d) = (wide(src), wide(dst));
    // MOVEFILE_REPLACE_EXISTING 을 쓰지 않으므로 대상이 있으면 OS 가 거부한다
    let flags = if write_through {
        MOVEFILE_WRITE_THROUGH
    } else {
        0
    };
    // SAFETY: s, d 는 NUL 로 끝나는 유효한 UTF-16 버퍼이고 호출 동안 살아 있다.
    if unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), flags) } != 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error().map(|c| c as u32) {
        Some(ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS) => Err(io::ErrorKind::AlreadyExists.into()),
        _ => Err(e),
    }
}

/// 비 Windows: 하드 링크는 대상이 있으면 실패하므로 link + unlink 로 덮어쓰기를 피한다.
#[cfg(not(windows))]
pub(super) fn move_no_replace(src: &Path, dst: &Path, _write_through: bool) -> io::Result<()> {
    match std::fs::hard_link(src, dst) {
        Ok(()) => std::fs::remove_file(src),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) if dst.exists() => Err(io::ErrorKind::AlreadyExists.into()),
        Err(_) => std::fs::rename(src, dst),
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Business rule (덮어쓰기 금지) / Concurrency (확인 이후 생성된 대상)
    use super::*;

    #[test]
    fn t_prot_12_move_succeeds_when_absent() {
        let d = tempfile::TempDir::new().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"data").unwrap();
        move_no_replace(&a, &b, true).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"data");
    }

    #[test]
    fn t_prot_12b_move_never_replaces_an_existing_target() {
        let d = tempfile::TempDir::new().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"mine").unwrap();
        std::fs::write(&b, b"theirs").unwrap(); // 존재 확인 이후에 생긴 파일을 흉내
        for wt in [false, true] {
            let e = move_no_replace(&a, &b, wt).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        }
        assert_eq!(std::fs::read(&a).unwrap(), b"mine");
        assert_eq!(std::fs::read(&b).unwrap(), b"theirs");
    }

    #[test]
    fn t_prot_12c_missing_source_is_a_plain_io_error() {
        let d = tempfile::TempDir::new().unwrap();
        let e = move_no_replace(&d.path().join("nope"), &d.path().join("b"), false).unwrap_err();
        assert_ne!(e.kind(), io::ErrorKind::AlreadyExists);
    }
}
