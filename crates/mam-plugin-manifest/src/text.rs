//! 플러그인이 준 표시·로그용 문자열 정리 — 제어 문자(줄바꿈 포함)는 공백으로, 서식 문자(Unicode Cf: 방향 재정의 U+202E,
//! 제로폭 문자 등)는 지운다. 로그 줄 위조와 이름 위장(RTL 재정의)을 막는다.

/// 유니코드 서식 문자(Cf) — 표시 폭이 없거나 방향·모양을 바꾼다
pub fn is_format_char(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// 제어 문자·줄/문단 구분자 -> 공백, 서식 문자 -> 제거
pub fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| !is_format_char(*c))
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation(제어·서식 문자) / Boundary(빈 문자열·일반 텍스트 불변)

    #[test]
    fn t_pmanifest_txt_01_strips_control_and_format_chars() {
        assert_eq!(clean_text("a\nb\r\nc\td\u{0}e"), "a b  c d e");
        assert_eq!(clean_text("evil\u{202E}gpj.exe"), "evilgpj.exe");
        assert_eq!(clean_text("a\u{200B}b\u{FEFF}c\u{2066}d\u{E0041}"), "abcd");
        assert_eq!(clean_text("x\u{2028}y\u{2029}z"), "x y z");
        assert_eq!(clean_text(""), "");
    }

    #[test]
    fn t_pmanifest_txt_02_keeps_normal_text() {
        let s = "EXIF 요약 — 日本語 café 🙂";
        assert_eq!(clean_text(s), s);
    }
}
