//! 폴더 -> 검증된 패키지 읽기. 설치 전 점검(`inspect_dir`)과 부팅 시 로드가 같은 경로를 쓴다.
//! 읽는 파일은 `plugin.json`, `l10n/*.json`, `main` 뿐이다. 크기는 **읽기 전에** 메타데이터로 확인하고, 실제 위치가 폴더 밖이면
//! (링크·정션) 거부한다. 오류 메시지에 호스트 경로를 싣지 않는다.

use std::fs;
use std::path::{Path, PathBuf};

use mam_core::storage::is_same_or_inside;
use mam_plugin_manifest::limits::{MAX_LOCALE_BYTES, MAX_MANIFEST_BYTES};
use mam_plugin_manifest::{
    L10N_DIR, LocaleBundle, LocaleError, MANIFEST_FILE_NAME, Manifest, ManifestError, check_engine,
    parse_locale_file, parse_manifest,
};
use semver::Version;

use crate::api::InstallError;

pub(crate) const MAX_LOCALE_FILES: usize = 64;
pub(crate) const MAX_MAIN_BYTES: u64 = 2 << 20;
/// `l10n` 폴더에서 훑어볼 항목 수 상한 (json 이 아닌 파일이 수없이 있어도 끝나게)
const MAX_L10N_ENTRIES: usize = 1_000;

/// 읽은 내용을 그대로 들고 있다 — 설치는 이 바이트를 복사한다(검증한 것과 설치되는 것이 같다).
pub(crate) struct Package {
    pub manifest: Manifest,
    pub manifest_text: String,
    /// (로케일 코드 = 파일 stem, 원문)
    pub locale_files: Vec<(String, String)>,
    pub locales: LocaleBundle,
    pub main_rel: PathBuf,
    pub main_source: String,
}

fn short(s: &str) -> String {
    s.chars().take(64).collect()
}

fn invalid_utf8() -> ManifestError {
    ManifestError::Json {
        line: 1,
        column: 1,
        message: "file is not valid UTF-8".to_owned(),
    }
}

/// 폴더를 읽는다. `app` 이 있으면 `engines.mam` 을 확인한다(부팅은 `None` — Incompatible 로 구분해 싣는다).
pub(crate) fn read_package(dir: &Path, app: Option<&Version>) -> Result<Package, InstallError> {
    let canon_dir = fs::canonicalize(dir).map_err(|_| InstallError::NotADirectory)?;
    let manifest_text = read_manifest_text(dir, &canon_dir)?;
    let manifest = parse_manifest(&manifest_text)?;
    if let Some(app) = app {
        check_engine(&manifest, app)?;
    }
    let (locale_files, locales) = read_locales(dir, &canon_dir)?;
    let (main_rel, main_source) = read_main(dir, &canon_dir, &manifest)?;
    Ok(Package {
        manifest,
        manifest_text,
        locale_files,
        locales,
        main_rel,
        main_source,
    })
}

/// 일반 파일이고 실제 위치가 `canon_dir` 안인가 (링크·정션 탈출 차단)
fn regular_inside(path: &Path, canon_dir: &Path) -> Option<fs::Metadata> {
    let meta = fs::symlink_metadata(path)
        .ok()
        .filter(fs::Metadata::is_file)?;
    let real = fs::canonicalize(path).ok()?;
    is_same_or_inside(&real, canon_dir).then_some(meta)
}

fn read_manifest_text(dir: &Path, canon_dir: &Path) -> Result<String, InstallError> {
    let path = dir.join(MANIFEST_FILE_NAME);
    let meta = regular_inside(&path, canon_dir).ok_or(InstallError::ManifestMissing)?;
    if meta.len() > MAX_MANIFEST_BYTES as u64 {
        let len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
        return Err(ManifestError::TooLarge {
            len,
            max: MAX_MANIFEST_BYTES,
        }
        .into());
    }
    let too_large = || ManifestError::TooLarge {
        len: MAX_MANIFEST_BYTES + 1,
        max: MAX_MANIFEST_BYTES,
    };
    let bytes = read_bounded(&path, MAX_MANIFEST_BYTES as u64)?.ok_or_else(too_large)?;
    String::from_utf8(bytes).map_err(|_| invalid_utf8().into())
}

fn read_locales(
    dir: &Path,
    canon_dir: &Path,
) -> Result<(Vec<(String, String)>, LocaleBundle), InstallError> {
    let (mut files, mut bundle) = (Vec::new(), LocaleBundle::default());
    let l10n = dir.join(L10N_DIR);
    // 폴더 자체가 링크·정션이면 없는 것으로 본다
    if !fs::symlink_metadata(&l10n).is_ok_and(|m| m.is_dir()) {
        return Ok((files, bundle));
    }
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    for (seen, entry) in fs::read_dir(&l10n)?.enumerate() {
        if seen >= MAX_L10N_ENTRIES {
            return Err(InstallError::TooManyLocaleFiles);
        }
        let path = entry?.path();
        let stem = (path.extension().is_some_and(|e| e == "json"))
            .then(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
            .flatten();
        if let Some(stem) = stem {
            candidates.push((stem, path));
        }
    }
    if candidates.len() > MAX_LOCALE_FILES {
        return Err(InstallError::TooManyLocaleFiles);
    }
    candidates.sort();
    for (stem, path) in candidates {
        let loc_err = |source: LocaleError| InstallError::Locale {
            file_stem: short(&stem),
            source,
        };
        let Some(meta) = regular_inside(&path, canon_dir) else {
            continue;
        };
        if meta.len() > MAX_LOCALE_BYTES as u64 {
            return Err(loc_err(LocaleError::TooLarge));
        }
        let bytes = read_bounded(&path, MAX_LOCALE_BYTES as u64)?
            .ok_or_else(|| loc_err(LocaleError::TooLarge))?;
        let text = String::from_utf8(bytes).map_err(|_| {
            loc_err(LocaleError::Json {
                line: 1,
                column: 1,
                message: "file is not valid UTF-8".to_owned(),
            })
        })?;
        bundle
            .insert(&stem, parse_locale_file(&text).map_err(&loc_err)?)
            .map_err(&loc_err)?;
        files.push((stem, text));
    }
    Ok((files, bundle))
}

fn read_main(
    dir: &Path,
    canon_dir: &Path,
    manifest: &Manifest,
) -> Result<(PathBuf, String), InstallError> {
    let rel = manifest.main.to_relative_path();
    let real = fs::canonicalize(dir.join(&rel)).map_err(|_| InstallError::MainMissing)?;
    if !is_same_or_inside(&real, canon_dir) {
        return Err(InstallError::MainEscapes);
    }
    let meta = fs::metadata(&real)
        .ok()
        .filter(fs::Metadata::is_file)
        .ok_or(InstallError::MainMissing)?;
    if meta.len() > MAX_MAIN_BYTES {
        return Err(InstallError::MainTooLarge);
    }
    let bytes = read_bounded(&real, MAX_MAIN_BYTES)?.ok_or(InstallError::MainTooLarge)?;
    let source = String::from_utf8(bytes).map_err(|_| InstallError::MainNotUtf8)?;
    Ok((rel, source))
}

/// 최대 `max` 바이트만 읽는다 — 메타데이터 검사 뒤 파일이 커져도(TOCTOU) 메모리를 무제한 쓰지 않는다. `max` 를 넘으면 `None`
pub(crate) fn read_bounded(path: &Path, max: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= max).then_some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Boundary(상한 정확히/1 초과) / External(없는 파일)

    #[test]
    fn t_pinst_pkg_01_read_bounded_accepts_up_to_max_and_rejects_beyond() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f.bin");
        std::fs::write(&p, vec![b'a'; 10]).unwrap();
        assert_eq!(read_bounded(&p, 10).unwrap().map(|b| b.len()), Some(10));
        assert_eq!(read_bounded(&p, 11).unwrap().map(|b| b.len()), Some(10));
        assert!(
            read_bounded(&p, 9).unwrap().is_none(),
            "one byte over the bound"
        );
        assert!(read_bounded(&p, 0).unwrap().is_none());
        std::fs::write(&p, b"").unwrap();
        assert_eq!(read_bounded(&p, 0).unwrap().map(|b| b.len()), Some(0));
        assert!(read_bounded(&tmp.path().join("missing"), 10).is_err());
    }

    #[test]
    fn t_pinst_pkg_02_read_bounded_stops_reading_at_the_bound() {
        // 상한보다 훨씬 큰 파일도 max+1 바이트까지만 읽고 거부한다 (무제한 읽기 없음)
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("big.bin");
        std::fs::write(&p, vec![b'z'; 1 << 20]).unwrap();
        assert!(read_bounded(&p, 1024).unwrap().is_none());
    }
}
