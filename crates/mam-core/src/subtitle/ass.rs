//! ASS/SSA 파서 — 텍스트만 (스타일·위치·효과 버림).

use super::{Cue, text};

pub(super) fn parse(src: &str) -> Vec<Cue> {
    let mut in_events = false;
    let mut cols: Option<(usize, usize, usize, usize)> = None; // start, end, text, 필드 수
    let mut cues = Vec::new();
    for line in text::normalize_newlines(src).lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(f) = strip_ci(line, "format:") {
            let names: Vec<String> = f.split(',').map(|s| s.trim().to_lowercase()).collect();
            let pos = |n: &str| names.iter().position(|x| x == n);
            cols = match (pos("start"), pos("end"), pos("text")) {
                (Some(s), Some(e), Some(t)) => Some((s, e, t, names.len())),
                _ => None,
            };
        } else if let (Some(d), Some((s, e, t, n))) = (strip_ci(line, "dialogue:"), cols) {
            let fields: Vec<&str> = d.splitn(n, ',').collect();
            if fields.len() < n {
                continue;
            }
            let (Some(start_ms), Some(end_ms)) = (
                text::parse_clock(fields[s].trim()),
                text::parse_clock(fields[e].trim()),
            ) else {
                continue;
            };
            if let Some(body) = dialogue_text(fields[t]) {
                cues.push(Cue {
                    start_ms,
                    end_ms,
                    text: body,
                });
            }
        }
    }
    text::finalize(cues)
}

fn strip_ci<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let head = line.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| line[prefix.len()..].trim_start())
}

/// override 블록 제거·`\N`/`\h` 치환. 그리기 명령(`\p1`~)이면 `None`.
fn dialogue_text(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut block = String::new();
                for b in chars.by_ref() {
                    if b == '}' {
                        break;
                    }
                    block.push(b);
                }
                if is_drawing(&block) {
                    return None;
                }
            }
            '\\' => match chars.peek() {
                Some('N' | 'n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('h') => {
                    chars.next();
                    out.push(' ');
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    Some(text::clean_lines(&out))
}

fn is_drawing(block: &str) -> bool {
    block
        .match_indices("\\p")
        .any(|(i, _)| matches!(block.as_bytes().get(i + 2), Some(b'1'..=b'9')))
}
