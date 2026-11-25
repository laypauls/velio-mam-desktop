//! 구독 로그인 URL 을 시스템 기본 브라우저로 여는 구현 — 프론트에는 노출하지 않는다(UI 는 URL 을 지정할 수 없고,
//! `mam_ai::external::cli::CliLogin` 이 CLI 출력에서 잡은 `https://` 인증 URL 만 이 함수에 넘긴다).
//! 호출 직전에 한 번 더 `https://` 와 제어 문자 없음을 확인한다(깊이 방어).

use std::io;

use mam_ai::external::cli::UrlOpener;

pub struct ShellUrlOpener;

/// 시스템 브라우저로 열어도 되는 URL 인가.
pub(crate) fn openable(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() > "https://".len()
        && url.len() <= 4096
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

impl UrlOpener for ShellUrlOpener {
    fn open(&self, url: &str) -> io::Result<()> {
        if !openable(url) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "url not allowed",
            ));
        }
        #[cfg(windows)]
        {
            use mam_core::reuse::{Launcher, ShellLauncher};
            ShellLauncher.open(std::path::Path::new(url))
        }
        #[cfg(not(windows))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no browser launcher",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation / Business rule(https 만)
    #[test]
    fn t_aicli_open_01_only_plain_https_urls_are_openable() {
        assert!(openable(
            "https://claude.com/cai/oauth/authorize?code=true&x=1"
        ));
        for bad in [
            "http://claude.com/oauth/authorize",
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "https://",
            "https://a b",
            "https://a\nb",
            "C:\\Windows\\System32\\cmd.exe",
            "",
        ] {
            assert!(!openable(bad), "{bad:?}");
        }
    }

    #[test]
    fn t_aicli_open_02_opener_rejects_disallowed_urls_without_launching() {
        let err = ShellUrlOpener.open("http://example.com").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
