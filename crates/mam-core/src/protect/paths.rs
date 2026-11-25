//! 경로 포함 관계 비교 (초기화·이동 공용). 심볼릭 링크·정션·`\?\` 접두어·대소문자 차이로 우회되지 않도록
//! 비교 전에 정규화(canonicalize)한다. 존재하지 않는 꼬리는 가장 가까운 존재하는 조상까지만 정규화한다.

use std::path::{Path, PathBuf};

use crate::volume::strip_verbatim;

/// 실제 경로로 해석한다. 실패하면(없는 경로) 존재하는 조상을 해석하고 나머지를 이어 붙인다.
pub(super) fn resolve(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut tail = Vec::new();
    let mut cur = p;
    while let (Some(name), Some(parent)) = (cur.file_name(), cur.parent()) {
        tail.push(name.to_owned());
        cur = parent;
        if let Ok(mut out) = std::fs::canonicalize(cur) {
            out.extend(tail.iter().rev());
            return out;
        }
    }
    p.to_path_buf()
}

fn norm(p: &Path) -> String {
    strip_verbatim(&resolve(p).to_string_lossy())
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// `child` 가 `parent` 와 같거나 그 안에 있다 (구성 요소 단위, 대소문자 무시)
pub(super) fn is_within(child: &Path, parent: &Path) -> bool {
    let (c, p) = (norm(child), norm(parent));
    c == p || c.strip_prefix(&p).is_some_and(|r| r.starts_with('\\'))
}

pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary / Business rule
    use super::*;

    #[test]
    fn t_prot_13_is_within_is_component_wise_case_insensitive_and_handles_missing_tails() {
        let d = tempfile::TempDir::new().unwrap();
        let root = d.path().join("Media");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase());
        assert!(is_within(&root.join("sub"), &root));
        assert!(is_within(&root, &root));
        assert!(is_within(&root.join("sub"), &upper));
        assert!(is_within(&root.join("not").join("yet"), &root)); // 없는 꼬리
        assert!(!is_within(&d.path().join("Media2"), &root)); // 문자열 접두어 != 포함
        assert!(!is_within(d.path(), &root));
        assert!(is_within(&root.join("sub").join("..").join("sub"), &root));
    }
}
