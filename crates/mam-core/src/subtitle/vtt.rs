//! WebVTT(.vtt) 파서 — `WEBVTT` 헤더가 없으면 빈 결과.

use super::{Cue, text};

pub(super) fn parse(src: &str) -> Vec<Cue> {
    let norm = text::normalize_newlines(src);
    let Some(after) = norm.strip_prefix("WEBVTT") else {
        return Vec::new();
    };
    if !after.is_empty() && !after.starts_with([' ', '\t', '\n']) {
        return Vec::new();
    }
    let cues = text::split_blocks(&norm)
        .iter()
        .skip(1) // 헤더 블록
        .filter(|b| {
            !["NOTE", "STYLE", "REGION"]
                .iter()
                .any(|k| b[0].starts_with(k))
        })
        .filter_map(|b| text::block_cue(b))
        .collect();
    text::finalize(cues)
}
