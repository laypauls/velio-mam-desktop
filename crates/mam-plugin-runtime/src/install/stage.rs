//! 패키지 복사 — `plugins/.staging/<id>-<n>/` 에 검증한 바이트만 쓰고 `plugins/<id>/<version>/` 로 rename 한다.
//! 같은 버전 재설치는 옛 폴더를 `.trash` 로 옮겼다가 설정 커밋 뒤에 지운다 (실패하면 되돌린다). 원본 폴더는 건드리지 않는다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mam_plugin_manifest::{L10N_DIR, MANIFEST_FILE_NAME};

use super::fsx::remove_confined;
use super::package::Package;
use crate::api::InstallError;

pub(crate) const STAGING_DIR: &str = ".staging";
pub(crate) const TRASH_DIR: &str = ".trash";

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// 설치 후 설정 커밋 전까지 되돌릴 수 있게 들고 있는 상태
pub(crate) struct Promoted {
    pub final_dir: PathBuf,
    trash: Option<PathBuf>,
}

/// 검증한 내용(plugin.json·l10n/*.json·main)만 스테이징 폴더에 쓴다. 실패하면 스테이징을 지운다
pub(crate) fn write_staging(
    plugins_dir: &Path,
    id: &str,
    pkg: &Package,
) -> Result<PathBuf, InstallError> {
    let staging_root = plugins_dir.join(STAGING_DIR);
    fs::create_dir_all(&staging_root)?;
    let dir = staging_root.join(unique(id));
    fs::create_dir(&dir)?;
    if let Err(e) = write_files(&dir, pkg) {
        let _ = remove_confined(plugins_dir, &dir);
        return Err(e.into());
    }
    Ok(dir)
}

fn write_files(dir: &Path, pkg: &Package) -> std::io::Result<()> {
    fs::write(dir.join(MANIFEST_FILE_NAME), &pkg.manifest_text)?;
    if !pkg.locale_files.is_empty() {
        let l10n = dir.join(L10N_DIR);
        fs::create_dir(&l10n)?;
        for (stem, text) in &pkg.locale_files {
            fs::write(l10n.join(format!("{stem}.json")), text)?;
        }
    }
    let main = dir.join(&pkg.main_rel);
    if let Some(parent) = main.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(main, &pkg.main_source)
}

/// 스테이징 -> `plugins/<id>/<version>`. 같은 버전이 이미 있으면 `.trash` 로 옮겨 둔다
pub(crate) fn promote(
    plugins_dir: &Path,
    id: &str,
    version: &str,
    staged: &Path,
) -> Result<Promoted, InstallError> {
    let result = promote_inner(plugins_dir, id, version, staged);
    if result.is_err() {
        let _ = remove_confined(plugins_dir, staged);
    }
    result
}

fn promote_inner(
    plugins_dir: &Path,
    id: &str,
    version: &str,
    staged: &Path,
) -> Result<Promoted, InstallError> {
    let parent = plugins_dir.join(id);
    fs::create_dir_all(&parent)?;
    let final_dir = parent.join(version);
    let mut trash = None;
    if fs::symlink_metadata(&final_dir).is_ok() {
        let trash_root = plugins_dir.join(TRASH_DIR);
        fs::create_dir_all(&trash_root)?;
        let t = trash_root.join(unique(&format!("{id}-{version}")));
        fs::rename(&final_dir, &t)?;
        trash = Some(t);
    }
    if let Err(e) = fs::rename(staged, &final_dir) {
        if let Some(t) = &trash {
            let _ = fs::rename(t, &final_dir);
        }
        return Err(e.into());
    }
    Ok(Promoted { final_dir, trash })
}

impl Promoted {
    /// 설정 커밋 실패 — 새 폴더를 지우고 옛 폴더를 되돌린다
    pub(crate) fn rollback(self, plugins_dir: &Path) {
        let _ = remove_confined(plugins_dir, &self.final_dir);
        if let Some(t) = &self.trash {
            let _ = fs::rename(t, &self.final_dir);
        }
    }

    /// 설정 커밋 성공 — 옛 같은 버전 복사본을 지운다 (실패하면 부팅 정리가 지운다)
    pub(crate) fn commit(self, plugins_dir: &Path) {
        if let Some(t) = &self.trash {
            let _ = remove_confined(plugins_dir, t);
        }
    }
}
