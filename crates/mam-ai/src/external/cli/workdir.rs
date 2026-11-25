//! 요청별 빈 작업 폴더 — CLI 의 cwd. 위치는 **OS 임시 폴더 아래**(`<temp>/mam-ai-cli/`)여야 한다.
//!
//! 실측(2.1.318): cwd 가 저장소 안이면 `--safe-mode` 를 써도 "Is a git repository" 환경 줄이 새고, 개발 환경처럼 MAMData 가
//! 저장소 안에 있으면 프로젝트 CLAUDE.md·git status 도 실렸다. MAMData 는 사용자가 어디든 고를 수 있어 안전하지 않으므로
//! MAMData 밖(임시 폴더)에 둔다. 원본 폴더가 cwd 가 되면 도구가 꺼져 있어도 위험하다. 내용은 CLI 가 남긴 것뿐이라 비어 있다.
//!
//! 앱 시작 시 `sweep` 이 비정상 종료로 남은 폴더를 치운다 (`CliHost::new`). 앱은 단일 인스턴스다.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 운영 작업 폴더의 부모 — `std::env::temp_dir()/mam-ai-cli`.
pub fn default_work_root() -> PathBuf {
    std::env::temp_dir().join("mam-ai-cli")
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 만든 폴더는 놓일 때 지운다 (내용은 CLI 가 쓴 것뿐이다).
#[derive(Debug)]
pub struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// `root` 아래에 새 빈 폴더를 만든다. 이름이 이미 있으면 실패한다(재사용 금지).
    pub fn create(root: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(root)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!(
            "run-{}-{}-{nanos:x}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let path = root.join(name);
        std::fs::create_dir(&path)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        // 실패해도 다음 시작 시 `sweep` 이 치운다
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 비정상 종료로 남은 작업 폴더를 모두 지운다 (앱 시작 시 한 번, 실행 중인 요청이 없을 때).
pub fn sweep(root: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(root) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("run-"))
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(재사용 금지) / Business rule(정리)
    #[test]
    fn t_aicli_wd_01_creates_unique_empty_dirs_under_root_and_removes_them() {
        let root = tempfile::tempdir().unwrap();
        let (a_path, b_path);
        {
            let a = WorkDir::create(root.path()).unwrap();
            let b = WorkDir::create(root.path()).unwrap();
            a_path = a.path().to_path_buf();
            b_path = b.path().to_path_buf();
            assert_ne!(a_path, b_path);
            assert!(a_path.starts_with(root.path()));
            assert_eq!(std::fs::read_dir(&a_path).unwrap().count(), 0);
        }
        assert!(!a_path.exists() && !b_path.exists());
        assert!(root.path().exists(), "root 는 남는다");
    }

    #[test]
    fn t_aicli_wd_02_drop_removes_files_the_cli_left_inside() {
        let root = tempfile::tempdir().unwrap();
        let p;
        {
            let w = WorkDir::create(root.path()).unwrap();
            std::fs::write(w.path().join("scratch.txt"), b"x").unwrap();
            p = w.path().to_path_buf();
        }
        assert!(!p.exists());
    }

    #[test]
    fn t_aicli_wd_03_sweep_removes_only_leftover_run_dirs() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("run-1-1-a")).unwrap();
        std::fs::create_dir(root.path().join("run-2-2-b")).unwrap();
        std::fs::create_dir(root.path().join("keep")).unwrap();
        assert_eq!(sweep(root.path()), 2);
        assert!(root.path().join("keep").exists());
        assert_eq!(sweep(&root.path().join("missing")), 0);
    }

    #[test]
    fn t_aicli_wd_05_default_root_is_under_the_os_temp_dir_and_named_for_mam() {
        let r = default_work_root();
        assert!(r.starts_with(std::env::temp_dir()));
        assert!(r.ends_with("mam-ai-cli"));
    }

    #[test]
    fn t_aicli_wd_04_create_fails_when_root_is_a_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        assert!(WorkDir::create(&file).is_err());
    }
}
