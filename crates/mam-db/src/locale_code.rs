//! UI 언어 코드 검증 — 언어 파일 이름(`<code>.json`)과 설정 `language` 가 같은 규칙을 쓴다.
//!
//! 형식: `^[a-z]{2,3}(-[A-Za-z0-9]{2,8}){0,3}$` (예: `ko`, `en`, `zh-TW`). 경로 구분자·`..`·`_` 는 통과할 수 없어
//! 파일 경로 조립에 써도 탈출하지 않는다.

/// 설정 `language` 기본값(내장 언어 `en`).
pub const DEFAULT_LANGUAGE: &str = "en";

/// 언어 코드 형식이 맞으면 `true`.
pub fn is_valid_locale_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    if !(2..=3).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    let mut subtags = 0;
    for p in parts {
        subtags += 1;
        if subtags > 3
            || !(2..=8).contains(&p.len())
            || !p.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary / Validation
    use super::*;

    #[test]
    fn t_locale_code_01_accepts_valid_tags() {
        for c in [
            "ko",
            "en",
            "zh-TW",
            "fil",
            "sr-Latn-RS",
            "es-419",
            "en-a1b2c3d4",
        ] {
            assert!(is_valid_locale_code(c), "{c}");
        }
    }

    #[test]
    fn t_locale_code_02_rejects_invalid_tags_and_traversal() {
        let too_many = "en-aa-bb-cc-dd";
        for c in [
            "",
            "e",
            "engl",
            "EN",
            "En",
            "en_US",
            "en-",
            "en-a",
            "en-123456789",
            "../x",
            "a\\b",
            "a/b",
            "..",
            "en-US/..",
            "en US",
            "한국",
            too_many,
        ] {
            assert!(!is_valid_locale_code(c), "{c:?}");
        }
    }
}
