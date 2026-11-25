//! `path_key` 접두 범위 — 디렉터리 하위 전체를 인덱스 범위 조회(`path_key >= lo AND path_key < hi`)로 가리킨다.
//!
//! 키는 `/` 구분이므로 `prefix/` 부터 `prefix0`(`/` 다음 문자 `0`) 미만까지가 `prefix` 의 하위다.
//! `LIKE` 는 인덱스를 못 쓰고 `%`/`_` 이스케이프 문제가 있어 쓰지 않는다. 빈 접두(볼륨 루트)는 전체 범위.

/// 빈 접두(전체)의 상한. 유효한 키는 모두 이보다 작다.
const MAX_KEY: &str = "\u{10FFFF}";

/// `prefix` 하위 경로의 `[lo, hi)` 범위. `prefix` 자신은 포함하지 않는다.
pub fn key_range(prefix: &str) -> (String, String) {
    if prefix.is_empty() {
        (String::new(), MAX_KEY.to_string())
    } else {
        (format!("{prefix}/"), format!("{prefix}0"))
    }
}

/// 폴더 루트 키와 루트 기준 상대 키를 볼륨 기준 키로 합친다 (`''` 는 해당 쪽이 없다는 뜻).
pub fn join_key(root: &str, rel: &str) -> String {
    match (root.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}/{rel}"),
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary / Business rule
    use super::*;

    #[test]
    fn t_scan_range_01_bounds_cover_descendants_only() {
        let (lo, hi) = key_range("a/b");
        assert!(lo.as_str() <= "a/b/c.jpg" && "a/b/c.jpg" < hi.as_str());
        assert!("a/b/z/y/x.png" < hi.as_str());
        assert!("a/b" < lo.as_str(), "the directory itself is excluded");
        assert!(
            "a/bc/x.jpg" >= hi.as_str(),
            "sibling with the same prefix is excluded"
        );
        assert!("a/b-x/1.jpg" < lo.as_str(), "'-' sorts before '/'");
    }

    #[test]
    fn t_scan_range_02_empty_prefix_covers_everything() {
        let (lo, hi) = key_range("");
        assert!(lo.as_str() <= "a.jpg" && "한글/x.jpg" < hi.as_str());
        assert_eq!(join_key("", "x"), "x");
        assert_eq!(join_key("r", ""), "r");
        assert_eq!(join_key("r", "x/y"), "r/x/y");
    }
}
