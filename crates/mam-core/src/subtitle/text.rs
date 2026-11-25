//! 파서 공용: 태그 제거·엔티티·줄 정리·시각 파싱·블록 분리·큐 정리.

use super::Cue;
use unicode_normalization::UnicodeNormalization;

/// 이름 비교 키: NFC + 소문자
pub(super) fn key(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// `<br>` 은 줄바꿈, 그 밖의 태그는 제거, 엔티티 디코드, 줄 정리.
pub(super) fn clean_markup(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let looks_tag = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '!' | '?'));
        match (looks_tag, after.find('>')) {
            (true, Some(j)) => {
                let inner = &after[..j];
                let name = inner
                    .split(|c: char| c.is_whitespace() || c == '/')
                    .find(|s| !s.is_empty());
                if !inner.starts_with('/') && name.is_some_and(|n| n.eq_ignore_ascii_case("br")) {
                    out.push('\n');
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    clean_lines(&decode_entities(&out))
}

/// 줄 앞뒤 공백 정리 + 빈 줄 제거
pub(super) fn clean_lines(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let decoded = after
            .find(';')
            .filter(|&j| j <= 10)
            .and_then(|j| entity(&after[..j]).map(|c| (c, j)));
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &after[j + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "nbsp" => Some(' '),
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// `[H+:]MM:SS[,.]fff` → ms. 소수부는 자릿수와 무관하게 분수로 해석(`5` = 500 ms).
pub(super) fn parse_clock(s: &str) -> Option<i64> {
    let (hms, frac) = match s.rfind([',', '.']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ms: i64 = 0;
    for (n, ch) in frac.chars().take(3).enumerate() {
        ms += i64::from(ch.to_digit(10)?) * [100, 10, 1][n];
    }
    let parts: Vec<&str> = hms.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut secs: i64 = 0;
    for (n, p) in parts.iter().enumerate() {
        let limit = if n == 0 && parts.len() == 3 { 3 } else { 2 };
        if p.is_empty() || p.len() > limit || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        secs = secs * 60 + p.parse::<i64>().ok()?;
    }
    Some(secs * 1000 + ms)
}

pub(super) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// 빈 줄 기준 블록 분리 (입력은 `normalize_newlines` 를 거친 것)
pub(super) fn split_blocks(s: &str) -> Vec<Vec<&str>> {
    let mut blocks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in s.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    blocks
}

/// SRT/WebVTT 공통 큐 블록: 앞 두 줄 안의 `a --> b` 줄. 깨진 블록은 `None`.
pub(super) fn block_cue(lines: &[&str]) -> Option<Cue> {
    let idx = lines.iter().take(2).position(|l| l.contains("-->"))?;
    let (a, b) = lines[idx].split_once("-->")?;
    let start_ms = parse_clock(a.trim())?;
    let end_ms = parse_clock(b.split_whitespace().next()?)?;
    let text = clean_markup(&lines[idx + 1..].join("\n"));
    Some(Cue {
        start_ms,
        end_ms,
        text,
    })
}

/// 빈 텍스트·`end <= start` 큐 제거, 시작 시각 기준 안정 정렬
pub(super) fn finalize(mut cues: Vec<Cue>) -> Vec<Cue> {
    cues.retain(|c| !c.text.is_empty() && c.end_ms > c.start_ms);
    cues.sort_by_key(|c| c.start_ms);
    cues
}
