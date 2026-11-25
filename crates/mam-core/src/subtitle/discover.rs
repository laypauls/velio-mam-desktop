//! 같은 폴더의 자막 후보 탐색 — 이름 매칭(stem·언어 접미사)과 상한 적용.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::lang::normalize_language;
use super::text::key;
use super::{MAX_CANDIDATES, MAX_SUBTITLE_BYTES, SubtitleFormat};

pub(super) struct Candidate {
    pub path: PathBuf,
    pub file_name: String,
    pub format: SubtitleFormat,
    /// 파일명 접미사(`영상.en.srt` 의 `en`)에서 얻은 언어
    pub suffix_lang: Option<&'static str>,
}

pub(super) fn discover(video: &Path) -> Vec<Candidate> {
    let Some(stem) = video.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let dir = match video.parent() {
        Some(p) if p.as_os_str().is_empty() => Path::new("."),
        Some(p) => p,
        None => return Vec::new(),
    };
    let names = match list_names(dir, &stem) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "subtitle: folder listing failed");
            return Vec::new();
        }
    };
    let stem_key = key(&stem);
    let mut found: Vec<Candidate> = names
        .into_iter()
        .filter_map(|name| {
            let (format, suffix_lang) = classify(&name, &stem_key)?;
            let path = dir.join(&name);
            let meta = fs::metadata(&path).ok().filter(fs::Metadata::is_file)?;
            if meta.len() > MAX_SUBTITLE_BYTES {
                tracing::warn!(path = %path.display(), "subtitle: file too large, skipped");
                return None;
            }
            Some(Candidate {
                path,
                file_name: name,
                format,
                suffix_lang,
            })
        })
        .collect();
    found.sort_by_cached_key(|c| (key(&c.file_name), c.file_name.clone()));
    found.dedup_by(|a, b| a.file_name == b.file_name);
    if found.len() > MAX_CANDIDATES {
        tracing::warn!(dir = %dir.display(), "subtitle: too many candidates, extras skipped");
        found.truncate(MAX_CANDIDATES);
    }
    found
}

/// 이름이 `stem.ext` 또는 `stem.<언어>.ext`(ext 는 지원 자막 확장자)면 형식·언어를 돌려준다
pub(super) fn classify(
    name: &str,
    stem_key: &str,
) -> Option<(SubtitleFormat, Option<&'static str>)> {
    let (base, ext) = name.rsplit_once('.')?;
    let format = match ext.to_ascii_lowercase().as_str() {
        "srt" => SubtitleFormat::Srt,
        "smi" | "sami" => SubtitleFormat::Smi,
        "vtt" => SubtitleFormat::Vtt,
        "ass" | "ssa" => SubtitleFormat::Ass,
        _ => return None,
    };
    if key(base) == stem_key {
        return Some((format, None));
    }
    let (front, suffix) = base.rsplit_once('.')?;
    if key(front) != stem_key {
        return None;
    }
    Some((format, Some(normalize_language(suffix)?)))
}

#[cfg(windows)]
fn list_names(dir: &Path, stem: &str) -> io::Result<Vec<String>> {
    super::discover_win::list(dir, stem)
}

#[cfg(not(windows))]
fn list_names(dir: &Path, _stem: &str) -> io::Result<Vec<String>> {
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .collect())
}
