//! SAMI(.smi/.sami) 파서 — 클래스(언어)마다 트랙 1개.

use std::collections::HashMap;

use super::lang::{normalize_language, sami_class_language};
use super::{Cue, Group, MAX_CUES, text};

/// 마지막 큐의 길이 (다음 SYNC 가 없을 때)
const LAST_CUE_MS: i64 = 5000;

struct Entry {
    class: String,
    start: i64,
    text: String,
}

/// 파일당 클래스(트랙) 수 상한 — 초과 시 파일 제외
pub(super) const MAX_SAMI_CLASSES: usize = 16;

/// `None` = 제외 대상 (클래스 수 또는 큐 수 상한 초과)
pub(super) fn parse(src: &str, suffix_lang: Option<&'static str>) -> Option<Vec<Group>> {
    let lower = src.to_ascii_lowercase(); // 바이트 위치가 src 와 같다
    let syncs = tag_positions(&lower, "<sync");
    let Some(&first) = syncs.first() else {
        return Some(Vec::new());
    };
    let head = &src[..first];
    let mut entries = Vec::new();
    // 클래스별 항목 인덱스 (등장 순서 유지)
    let mut order: Vec<String> = Vec::new();
    let mut by_class: HashMap<String, Vec<usize>> = HashMap::new();
    for (n, &pos) in syncs.iter().enumerate() {
        let end = syncs.get(n + 1).copied().unwrap_or(src.len());
        let from = entries.len();
        sync_entries(&src[pos..end], &mut entries);
        for (i, e) in entries.iter().enumerate().skip(from) {
            by_class
                .entry(e.class.clone())
                .or_insert_with(|| {
                    order.push(e.class.clone());
                    Vec::new()
                })
                .push(i);
        }
        if order.len() > MAX_SAMI_CLASSES || entries.len() > MAX_CUES * MAX_SAMI_CLASSES {
            return None;
        }
    }

    let mut groups = Vec::with_capacity(order.len());
    for class in order {
        let mine = &by_class[&class];
        if mine.len() > MAX_CUES + 1 {
            return None;
        }
        let cues = mine
            .iter()
            .enumerate()
            .filter(|&(_, &k)| !entries[k].text.is_empty())
            .map(|(i, &k)| {
                let e = &entries[k];
                Cue {
                    start_ms: e.start,
                    end_ms: mine
                        .get(i + 1)
                        .map_or(e.start.saturating_add(LAST_CUE_MS), |&nx| entries[nx].start),
                    text: e.text.clone(),
                }
            })
            .collect();
        let language = sami_class_language(&class)
            .or_else(|| style_language(head, &class))
            .or(suffix_lang);
        groups.push(Group {
            class: Some(class),
            language,
            cues: text::finalize(cues),
        });
    }
    Some(groups)
}

/// `name`(소문자, `<` 포함) 바로 뒤가 공백 또는 `>` 인 위치들
fn tag_positions(lower: &str, name: &str) -> Vec<usize> {
    lower
        .match_indices(name)
        .map(|(i, _)| i)
        .filter(|&i| {
            lower
                .as_bytes()
                .get(i + name.len())
                .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
        })
        .collect()
}

/// `<SYNC Start=N>` 한 덩어리에서 `<P Class=X>` 항목을 뽑는다
fn sync_entries(chunk: &str, out: &mut Vec<Entry>) {
    let Some(gt) = chunk.find('>') else { return };
    let Some(start) = attr(&chunk[5..gt], "start").and_then(|v| v.parse::<i64>().ok()) else {
        return;
    };
    let body = &chunk[gt + 1..];
    let ps = tag_positions(&body.to_ascii_lowercase(), "<p");
    for (n, &p) in ps.iter().enumerate() {
        let Some(close) = body[p..].find('>') else {
            continue;
        };
        let class = attr(&body[p + 2..p + close], "class").unwrap_or_default();
        let end = ps.get(n + 1).copied().unwrap_or(body.len());
        out.push(Entry {
            class: class.to_ascii_uppercase(),
            start,
            text: text::clean_markup(&body[(p + close + 1).min(end)..end]),
        });
    }
}

/// 태그 속성 값 (`name = "v"` / `name=v`, 이름은 대소문자 무시)
fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let at = lower.match_indices(name).find_map(|(i, _)| {
        let before_ok = i == 0 || !lower.as_bytes()[i - 1].is_ascii_alphanumeric();
        let rest = lower[i + name.len()..].trim_start();
        (before_ok && rest.starts_with('=')).then_some(i)
    })?;
    let rest = attrs[at + name.len()..]
        .trim_start()
        .trim_start_matches('=')
        .trim_start()
        .trim_start_matches(['"', '\'']);
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '>'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// `<STYLE>` 의 `.CLASS { lang: xx-YY; Name: ... }` 에서 언어 (`lang:` → `Name:` 순)
fn style_language(head: &str, class: &str) -> Option<&'static str> {
    let mut rest = head;
    while let Some(open) = rest.find('{') {
        let close = open + rest[open..].find('}')?;
        let selector = &rest[..open];
        let body = &rest[open + 1..close];
        rest = &rest[close + 1..];
        let name: String = selector
            .rfind('.')
            .map(|d| &selector[d + 1..])
            .unwrap_or_default()
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
            .collect();
        if name.eq_ignore_ascii_case(class) {
            for key in ["lang", "name"] {
                if let Some(l) = property(body, key).and_then(normalize_language) {
                    return Some(l);
                }
            }
        }
    }
    None
}

fn property<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.split(';').find_map(|decl| {
        let (k, v) = decl.split_once(':')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
    })
}
