//! CF_HDROP 버퍼 구성 — 순수 함수라 OS 없이 시험한다.

use std::io;
use std::path::PathBuf;

use crate::volume::strip_verbatim;

/// `DROPFILES` 헤더 크기: pFiles(u32) + pt(i32 x2) + fNC(i32) + fWide(i32)
const HEADER_LEN: u32 = 20;

/// `DROPFILES` 헤더 + UTF-16 경로 목록(각각 NUL 종료) + 목록 종료 NUL. 복사(copy) 의미의 파일 목록이다.
pub fn build_drop_files(paths: &[PathBuf]) -> io::Result<Vec<u8>> {
    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no paths for clipboard",
        ));
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN as usize + paths.len() * 64);
    bytes.extend_from_slice(&HEADER_LEN.to_le_bytes());
    bytes.extend_from_slice(&[0u8; 12]); // pt.x, pt.y, fNC
    bytes.extend_from_slice(&1i32.to_le_bytes()); // fWide = UTF-16
    for p in paths {
        let text = strip_verbatim(&p.to_string_lossy());
        if text.is_empty() || text.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid path for clipboard",
            ));
        }
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes.extend_from_slice(&[0, 0]);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Validation
    use super::*;

    fn utf16(bytes: &[u8]) -> Vec<u16> {
        bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn t_reuse_10_drop_files_layout_is_header_then_nul_separated_wide_paths() {
        // Arrange
        let paths = vec![
            PathBuf::from(r"D:\a\한글.jpg"),
            PathBuf::from(r"\\?\E:\b.mp4"),
        ];

        // Act
        let bytes = build_drop_files(&paths).unwrap();

        // Assert: 헤더(오프셋 20, fWide=1) + "경로\0경로\0\0"
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(i32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        let body = utf16(&bytes[20..]);
        let text = String::from_utf16(&body).unwrap();
        assert_eq!(text, "D:\\a\\한글.jpg\0E:\\b.mp4\0\0");
    }

    #[test]
    fn t_reuse_11_drop_files_rejects_empty_list_and_nul_in_path() {
        assert!(build_drop_files(&[]).is_err());
        assert!(build_drop_files(&[PathBuf::from("a\0b")]).is_err());
    }
}
