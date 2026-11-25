//! UI 언어 파일 폴더 — 고정 경로(`<bootstrap_dir>\locales`)의 `<code>.json` 을 나열·읽는다.
//!
//! 언어 파일은 사용자가 넣는 신뢰할 수 없는 입력이다: 파일명은 언어 코드 정규식으로, 내용은 크기(2 MiB)·JSON·
//! 모양(문자열/객체만)으로 검증한다. 읽기 전용이며 원본 미디어와 무관하다. 로그·에러 메시지에는 파일 내용을 싣지 않는다.
//! Rust <-> TS 계약: `src/lib/ipc.ts` (`list_locales`, `read_locale`, `open_locale_folder`).

pub mod commands;
mod files;
pub mod seed;
#[cfg(test)]
mod seed_tests;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

/// 부트스트랩 폴더 아래 언어 폴더 이름 (고정 — 설정·환경 변수 재정의 없음).
pub const LOCALES_DIR_NAME: &str = "locales";

/// 언어 폴더 경로.
pub fn locales_dir(bootstrap_dir: &Path) -> PathBuf {
    bootstrap_dir.join(LOCALES_DIR_NAME)
}

/// 앱 시작 시 언어 폴더를 만든다(없을 때만). 실패해도 앱은 내장 영어로 동작하므로 경고만 남긴다.
pub fn ensure_locales_dir(bootstrap_dir: &Path) {
    let dir = locales_dir(bootstrap_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(%e, path = %dir.display(), "cannot create the locale folder");
    }
}
