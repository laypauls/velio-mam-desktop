//! 볼륨 기준 상대 경로의 비교 키.
//!
//! SQLite `NOCASE` 는 ASCII 만 처리하므로 비교 키는 Rust 에서 만든다.
//! 규칙: 구분자 `\` -> `/` 통일, 연속 구분자 압축, 유니코드 NFC, 대소문자 접기(`to_lowercase`),
//! 끝 구분자 제거. 길이는 자르지 않는다 (260자 초과 허용).
//!
//! - 선행 구분자: 일반 경로는 볼륨 상대 경로이므로 제거한다 (`/a` == `a`).
//!   UNC(`\\server\share\..`, `//server/share/..`)는 선행 `//` 를 유지한다.
//! - `.` / `..` 세그먼트는 해석하지 않고 그대로 둔다 (호출자가 절대 경로를 풀어서 넘긴다).
//! - `\\?\` 같은 확장 길이 접두어는 처리하지 않는다 (호출자가 제거해서 넘긴다).
//! - **빈 입력(또는 구분자만 있는 입력)은 오류** — 파일의 상대 경로는 비어 있을 수 없고,
//!   `file_locations.path_key` 도 `CHECK (path_key <> '')` 로 막는다.
//!
//! 잔여 위험: NTFS 는 볼륨별 대문자 변환표를 쓰고 폴더별 대소문자 구분을 켤 수 있어
//! Rust 의 유니코드 소문자화와 일부 문자에서 다를 수 있다 (P1-4 에서 실제 볼륨으로 확인).

use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathKeyError {
    #[error("path is empty")]
    Empty,
}

pub fn path_key(rel_path: &str) -> Result<String, PathKeyError> {
    let unified: String = rel_path.nfc().collect::<String>().replace('\\', "/");
    let is_unc = unified.starts_with("//");

    let segments: Vec<&str> = unified.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Err(PathKeyError::Empty);
    }

    let mut joined = segments.join("/");
    if is_unc {
        joined.insert_str(0, "//");
    }
    // 소문자화가 정규화 형태를 깨는 드문 경우를 위해 한 번 더 NFC
    Ok(joined.to_lowercase().nfc().collect())
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Validation
    //! AAA: 각 테스트에서 입력 리터럴(Arrange) -> path_key(Act) -> 기대 키 비교(Assert).

    use super::*;

    fn decompose(s: &str) -> String {
        s.nfd().collect()
    }

    #[test]
    fn t_db_10_case_is_folded() {
        assert_eq!(
            path_key("Photos/IMG_001.JPG").unwrap(),
            "photos/img_001.jpg"
        );
        assert_eq!(path_key("Ünï/Ä.png").unwrap(), "ünï/ä.png");
    }

    #[test]
    fn t_db_10_korean_nfd_equals_nfc() {
        let nfc = "사진/한글.jpg";
        let nfd = decompose(nfc);
        assert_ne!(nfc, nfd, "fixture must really be decomposed");
        assert_eq!(path_key(&nfd).unwrap(), path_key(nfc).unwrap());
    }

    #[test]
    fn t_db_10_mixed_separators_unified() {
        assert_eq!(path_key("a\\b/c\\d.txt").unwrap(), "a/b/c/d.txt");
        assert_eq!(path_key("a//b\\\\c").unwrap(), "a/b/c");
    }

    #[test]
    fn t_db_10_trailing_and_leading_separators_removed() {
        assert_eq!(path_key("a/b/").unwrap(), "a/b");
        assert_eq!(path_key("a\\b\\\\").unwrap(), "a/b");
        assert_eq!(path_key("/a/b").unwrap(), "a/b");
        assert_eq!(path_key("\\a\\b").unwrap(), "a/b");
    }

    #[test]
    fn t_db_10_empty_input_is_error() {
        assert_eq!(path_key(""), Err(PathKeyError::Empty));
        assert_eq!(path_key("/"), Err(PathKeyError::Empty));
        assert_eq!(path_key("\\\\"), Err(PathKeyError::Empty));
        assert_eq!(path_key("///"), Err(PathKeyError::Empty));
    }

    #[test]
    fn t_db_10_long_path_not_truncated() {
        let long = format!("{}/file.jpg", "Dir".repeat(120)); // > 260 chars
        assert!(long.len() > 260);
        let key = path_key(&long).unwrap();
        assert_eq!(key.len(), long.len());
        assert!(key.ends_with("/file.jpg"));
    }

    #[test]
    fn t_db_10_unc_keeps_leading_double_separator() {
        assert_eq!(
            path_key("\\\\NAS\\Share\\Dir\\F.jpg").unwrap(),
            "//nas/share/dir/f.jpg"
        );
        assert_eq!(path_key("//NAS/Share/").unwrap(), "//nas/share");
        // 선행 구분자가 3개 이상이어도 UNC 접두는 정확히 2개
        assert_eq!(path_key("\\\\\\NAS\\S").unwrap(), "//nas/s");
    }

    #[test]
    fn t_db_10_unc_differs_from_relative_path() {
        assert_ne!(path_key("\\\\nas\\s").unwrap(), path_key("nas/s").unwrap());
    }

    #[test]
    fn t_db_10_idempotent() {
        let once = path_key("\\\\NAS\\Share\\한글\\A.JPG").unwrap();
        assert_eq!(path_key(&once).unwrap(), once);
    }

    #[test]
    fn t_db_10_dot_segments_are_preserved() {
        assert_eq!(path_key("a/./b/../C").unwrap(), "a/./b/../c");
    }
}
