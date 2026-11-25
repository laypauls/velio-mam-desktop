//! 동영상 옆 외부 자막 — 탐색 → 디코드 → 파싱 → 정렬 (설계: architect-subtitles-design §1).
//!
//! 자막 파일은 `File::open`(읽기 전용)으로만 연다. 이 모듈은 아무것도 쓰지 않는다.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Serialize;

mod ass;
mod decode;
mod discover;
#[cfg(windows)]
mod discover_win;
mod lang;
mod sami;
mod srt;
mod text;
mod vtt;
mod write;

pub use lang::normalize_language;
pub use write::to_srt;

/// 자막 파일 크기 상한 — 초과 시 제외
pub const MAX_SUBTITLE_BYTES: u64 = 16 * 1024 * 1024;
/// 트랙당 큐 수 상한 — 초과 시 파일 제외
pub const MAX_CUES: usize = 100_000;
/// 후보 파일 수 상한
pub const MAX_CANDIDATES: usize = 32;
/// 트랙 id 최대 길이 (SAMI 클래스 이름을 자른다)
const MAX_CLASS_ID_CHARS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    Srt,
    Smi,
    Vtt,
    Ass,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cue {
    pub start_ms: i64,
    pub end_ms: i64,
    /// 일반 텍스트, 줄바꿈 `\n`
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    /// SAMI 는 `"{file_name}#{CLASS}"`(클래스 없음은 `#`), 그 외 `"{file_name}"`
    pub id: String,
    /// ISO 639-1, 모르면 `None`
    pub language: Option<String>,
    pub format: SubtitleFormat,
    /// 표시용 파일명 (경로 아님)
    pub file_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTrack {
    pub track: SubtitleTrack,
    pub cues: Vec<Cue>,
}

/// 파일 하나에서 나온 트랙 후보 (SAMI 는 클래스마다 1개)
struct Group {
    class: Option<String>,
    language: Option<&'static str>,
    cues: Vec<Cue>,
}

/// 영상과 같은 폴더의 자막 트랙을 정렬해서 돌려준다.
/// 오류를 돌려주지 않는다 — 폴더 읽기 실패·파일별 실패는 warn 로그(경로는 로그에만) 후 제외.
pub fn load_sidecar_tracks(video: &Path) -> Vec<ParsedTrack> {
    let mut all: Vec<ParsedTrack> = discover::discover(video)
        .iter()
        .flat_map(load_file)
        .collect();
    // 안정 정렬: 언어 있는 것 먼저 → 언어 코드 → 형식 → 파일명. 같은 파일의 클래스 순서는 유지
    all.sort_by_cached_key(|p| {
        (
            p.track.language.is_none(),
            p.track.language.clone().unwrap_or_default(),
            format_rank(p.track.format),
            text::key(&p.track.file_name),
        )
    });
    all
}

fn format_rank(f: SubtitleFormat) -> u8 {
    match f {
        SubtitleFormat::Srt => 0,
        SubtitleFormat::Vtt => 1,
        SubtitleFormat::Smi => 2,
        SubtitleFormat::Ass => 3,
    }
}

fn load_file(c: &discover::Candidate) -> Vec<ParsedTrack> {
    let warn = |msg: &str| tracing::warn!(path = %c.path.display(), "subtitle: {msg}");
    let Ok(file) = File::open(&c.path) else {
        warn("open failed");
        return Vec::new();
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_SUBTITLE_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        warn("read failed");
        return Vec::new();
    }
    if bytes.len() as u64 > MAX_SUBTITLE_BYTES {
        warn("file too large");
        return Vec::new();
    }
    let Some(src) = decode::decode(&bytes) else {
        warn("undecodable (not UTF-8/UTF-16/CP949)");
        return Vec::new();
    };
    let single = |cues| {
        vec![Group {
            class: None,
            language: c.suffix_lang,
            cues,
        }]
    };
    let groups = match c.format {
        SubtitleFormat::Srt => single(srt::parse(&src)),
        SubtitleFormat::Vtt => single(vtt::parse(&src)),
        SubtitleFormat::Ass => single(ass::parse(&src)),
        SubtitleFormat::Smi => match sami::parse(&src, c.suffix_lang) {
            Some(g) => g,
            None => {
                warn("too many SAMI classes or cues");
                return Vec::new();
            }
        },
    };
    if groups.iter().any(|g| g.cues.len() > MAX_CUES) {
        warn("too many cues");
        return Vec::new();
    }
    groups
        .into_iter()
        .filter(|g| !g.cues.is_empty())
        .map(|g| ParsedTrack {
            track: SubtitleTrack {
                id: match (&g.class, c.format) {
                    (Some(class), SubtitleFormat::Smi) => {
                        let class: String = class.chars().take(MAX_CLASS_ID_CHARS).collect();
                        format!("{}#{class}", c.file_name)
                    }
                    _ => c.file_name.clone(),
                },
                language: g.language.map(str::to_owned),
                format: c.format,
                file_name: c.file_name.clone(),
            },
            cues: g.cues,
        })
        .collect()
}
