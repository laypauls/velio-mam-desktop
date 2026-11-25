//! 파일 삭제 도우미 — 모든 삭제는 `plugins/` 폴더 안으로 한정한다 (링크·정션을 따라가 밖을 지우지 않는다).

use std::fs;
use std::io;
use std::path::Path;

use mam_core::storage::is_same_or_inside;

/// `target` 을 지운다. 없으면 성공. 링크(심볼릭·정션)는 링크 자체만 지우고, 그 밖의 대상은 `root` 안(루트 자신은 제외)일 때만 지운다.
pub(crate) fn remove_confined(root: &Path, target: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let ft = meta.file_type();
    if ft.is_symlink() {
        return fs::remove_file(target).or_else(|_| fs::remove_dir(target));
    }
    let outside = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "path is outside the plugins folder",
        )
    };
    let (root_c, target_c) = (fs::canonicalize(root)?, fs::canonicalize(target)?);
    if !is_same_or_inside(&target_c, &root_c) || is_same_or_inside(&root_c, &target_c) {
        return Err(outside());
    }
    if ft.is_dir() {
        fs::remove_dir_all(target)
    } else {
        fs::remove_file(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Business rule(경계 밖 삭제 거부) / Boundary(없는 경로)

    #[test]
    fn t_pinst_fsx_01_removes_inside_and_ignores_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("plugins");
        fs::create_dir_all(root.join("a").join("b")).unwrap();
        fs::write(root.join("a").join("b").join("f.txt"), "x").unwrap();

        remove_confined(&root, &root.join("a")).unwrap();
        remove_confined(&root, &root.join("missing")).unwrap();

        assert!(!root.join("a").exists());
    }

    #[test]
    fn t_pinst_fsx_02_refuses_root_itself_and_outside_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("plugins");
        let other = tmp.path().join("other");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("keep.txt"), "x").unwrap();

        assert!(remove_confined(&root, &root).is_err());
        assert!(remove_confined(&root, &other).is_err());
        assert!(remove_confined(&root, &root.join("..").join("other")).is_err());

        assert!(other.join("keep.txt").exists());
        assert!(root.exists());
    }
}
