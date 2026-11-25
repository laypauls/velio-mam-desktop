//! 로그인 명령 출력에서 OAuth 인증 URL 을 잡는다.
//!
//! - 모양(`.../oauth/authorize...`)으로 판별한다 — 안내 문구는 CLI 버전마다 바뀌지만 URL 모양은 프로토콜의 일부다
//! - **`https://` 만** 받는다 (`http://` 등은 무시) — 시스템 브라우저로 여는 URL 은 이 함수가 돌려준 값뿐이다
//! - ANSI 제어열 제거, 줄바꿈으로 끊긴 URL 이어 붙이기: `&`/`?` 로 시작하는 줄은 앞줄의 연속이다(선례 실측)

const MAX_URL_LEN: usize = 4096;
const SHAPE: &str = "oauth/authorize";

/// 출력 정규화: ANSI 제거, CR 을 LF 로 통일, 줄 머리의 `&`/`?` 연속을 앞줄에 붙인다.
pub fn normalize_output(raw: &str) -> String {
    let no_ansi = strip_ansi(raw).replace("\r\n", "\n").replace('\r', "\n");
    join_continuations(&no_ansi)
}

fn join_continuations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let lines: Vec<&str> = text.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        let is_cont = i > 0 && line.trim_start_matches([' ', '\t']).starts_with(['&', '?']);
        if is_cont {
            out.push_str(line.trim_start_matches([' ', '\t']));
        } else {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(line);
        }
    }
    out
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 정규화된 출력에서 **마지막** 인증 URL (`https://` 만, 길이·문자 검증).
pub fn last_authorize_url(normalized: &str) -> Option<String> {
    let mut found = None;
    let mut from = 0;
    while let Some(i) = normalized[from..].find("https://") {
        let start = from + i;
        let end = normalized[start..]
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>') || c.is_control()
            })
            .map_or(normalized.len(), |e| start + e);
        let token = &normalized[start..end];
        if token.len() <= MAX_URL_LEN && token.contains(SHAPE) && token.len() > "https://".len() {
            found = Some(token.to_string());
        }
        from = end.max(start + 1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=abc&response_type=code&redirect_uri=https%3A%2F%2Fexample%2Fcb&scope=user%3Ainference&state=xyz";

    fn extract(raw: &str) -> Option<String> {
        last_authorize_url(&normalize_output(raw))
    }

    // 분류: Happy / Boundary(줄바꿈) / Validation(https 만)
    #[test]
    fn t_aicli_login_url_01_extracts_url_from_prompt_text() {
        let raw = format!(
            "Opening browser to sign in...\nIf the browser didn't open, visit:\n{URL}\nPaste code here if prompted > "
        );
        assert_eq!(extract(&raw).as_deref(), Some(URL));
    }

    #[test]
    fn t_aicli_login_url_02_joins_url_wrapped_before_ampersand() {
        let (a, b) = URL.split_once("&redirect_uri").unwrap();
        let raw = format!("visit:\n{a}\n   &redirect_uri{b}\nPaste code here");
        assert_eq!(extract(&raw).as_deref(), Some(URL));
        let raw_q = "https://x.example/oauth/authorize\n?code=true&a=b";
        assert_eq!(
            extract(raw_q).as_deref(),
            Some("https://x.example/oauth/authorize?code=true&a=b")
        );
    }

    #[test]
    fn t_aicli_login_url_03_strips_ansi_and_carriage_returns() {
        let raw = format!("\u{1b}[1mvisit\u{1b}[0m\r\n\u{1b}[36m{URL}\u{1b}[0m\r\n");
        assert_eq!(extract(&raw).as_deref(), Some(URL));
    }

    #[test]
    fn t_aicli_login_url_04_ignores_non_https_and_non_authorize_urls() {
        assert_eq!(extract("http://claude.com/cai/oauth/authorize?x=1"), None);
        assert_eq!(extract("ftp://x/oauth/authorize"), None);
        assert_eq!(
            extract("see https://docs.example.com/help for details"),
            None
        );
        assert_eq!(extract("javascript:alert(1)"), None);
        assert_eq!(extract(""), None);
        assert_eq!(extract("https://"), None);
    }

    #[test]
    fn t_aicli_login_url_05_takes_the_last_url_when_the_cli_restarts_its_flow() {
        let second = URL.replace("state=xyz", "state=new");
        let raw = format!("{URL}\nInvalid code\n{second}\n");
        assert_eq!(extract(&raw).as_deref(), Some(second.as_str()));
    }

    #[test]
    fn t_aicli_login_url_06_oversized_token_is_rejected() {
        let long = format!(
            "https://x.example/oauth/authorize?{}",
            "a".repeat(MAX_URL_LEN)
        );
        assert_eq!(extract(&long), None);
    }
}
