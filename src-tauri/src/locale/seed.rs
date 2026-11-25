//! 동봉 언어팩 첫 실행 복사 — 설치 폴더 `locales\*.json` 을 언어 폴더로 **없는 파일만** 복사한다.
//!
//! 사용자가 고친 파일을 덮어쓰지 않는다(`create_new` 로 경쟁 상태도 방지). 실패는 보고만 하고 앱 시작을 막지 않는다.

use std::io::{self, Write};
use std::path::Path;

use mam_db::locale_code::is_valid_locale_code;

/// 복사 결과 — 파일명 목록만 담는다(경로·내용은 싣지 않는다).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeedReport {
    pub copied: Vec<String>,
    pub failed: Vec<String>,
}

/// `src_dir` 최상위의 유효한 `<code>.json` 중 `dst_dir` 에 없는 것만 복사한다. `src_dir` 이 없으면 아무것도 하지 않는다.
pub fn seed_bundled_locales(src_dir: &Path, dst_dir: &Path) -> SeedReport {
    let mut report = SeedReport::default();
    let Ok(entries) = std::fs::read_dir(src_dir) else {
        return report;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(code) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_valid_locale_code(code) || !path.is_file() {
            continue;
        }
        match copy_if_absent(&path, &dst_dir.join(name)) {
            Ok(true) => report.copied.push(name.to_owned()),
            Ok(false) => {}
            Err(e) => {
                tracing::warn!(%e, file = name, "cannot copy a bundled language pack");
                report.failed.push(name.to_owned());
            }
        }
    }
    report.copied.sort();
    report.failed.sort();
    report
}

/// 대상이 없을 때만 쓴다. 이미 있으면 `Ok(false)`. 쓰다 실패하면 반쯤 쓴 파일을 지운다.
fn copy_if_absent(src: &Path, dst: &Path) -> io::Result<bool> {
    let body = std::fs::read(src)?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    if let Err(e) = file.write_all(&body).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(dst);
        return Err(e);
    }
    Ok(true)
}
