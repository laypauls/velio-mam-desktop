//! Windows 후보 나열 — `FindFirstFileExW("dir\stem.*")`.
//!
//! SMB `QUERY_DIRECTORY` 가 패턴을 서버에서 걸러, 폴더 크기와 거의 무관하게 동작한다.
//! 패턴은 8.3 짧은 이름에도 매칭되므로(오탐) 호출자가 긴 이름으로 반드시 재검증한다.
//! 비 ASCII stem 은 NFC·NFD 패턴을 모두 조회한다.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use unicode_normalization::UnicodeNormalization;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
};

/// 파일(폴더 제외) 이름 목록. 중복 가능 — 호출자가 제거한다.
pub(super) fn list(dir: &Path, stem: &str) -> io::Result<Vec<String>> {
    let mut patterns = vec![stem.to_owned()];
    if !stem.is_ascii() {
        for alt in [stem.nfc().collect::<String>(), stem.nfd().collect()] {
            if !patterns.contains(&alt) {
                patterns.push(alt);
            }
        }
    }
    let mut names = Vec::new();
    for (i, p) in patterns.iter().enumerate() {
        // 첫 패턴(원래 stem) 실패만 오류로 보고, 이후 NFC/NFD 실패는 이미 찾은 결과를 유지한다
        if let Err(e) = search(dir, p, &mut names) {
            if i == 0 {
                return Err(e);
            }
            tracing::warn!(dir = %dir.display(), error = %e, "subtitle: alternate-form search failed");
        }
    }
    Ok(names)
}

fn search(dir: &Path, stem: &str, names: &mut Vec<String>) -> io::Result<()> {
    let pattern = dir.join(format!("{stem}.*"));
    let wide: Vec<u16> = pattern.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: WIN32_FIND_DATAW 는 모든 필드가 정수/배열인 POD 라 0 초기화가 유효하다.
    let mut data: WIN32_FIND_DATAW = unsafe { std::mem::zeroed() };
    // SAFETY: wide 는 NUL 종료 UTF-16 이고 data 는 유효한 출력 버퍼다. 읽기 전용 검색만 한다.
    let handle = unsafe {
        FindFirstFileExW(
            wide.as_ptr(),
            FindExInfoBasic,
            std::ptr::from_mut(&mut data).cast::<c_void>(),
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let err = io::Error::last_os_error();
        return if err.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
            Ok(())
        } else {
            Err(err)
        };
    }
    loop {
        if data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            let len = data
                .cFileName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(data.cFileName.len());
            names.push(String::from_utf16_lossy(&data.cFileName[..len]));
        }
        // SAFETY: handle 은 위에서 얻은 유효한 검색 핸들이다.
        if unsafe { FindNextFileW(handle, &mut data) } == 0 {
            break;
        }
    }
    // SAFETY: 위에서 연 핸들을 한 번만 닫는다.
    unsafe { FindClose(handle) };
    Ok(())
}
