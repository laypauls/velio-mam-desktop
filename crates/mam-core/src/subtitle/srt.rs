//! SubRip(.srt) 파서 — 깨진 블록만 건너뛴다.

use super::{Cue, text};

pub(super) fn parse(src: &str) -> Vec<Cue> {
    let norm = text::normalize_newlines(src);
    let cues = text::split_blocks(&norm)
        .iter()
        .filter_map(|b| text::block_cue(b))
        .collect();
    text::finalize(cues)
}
