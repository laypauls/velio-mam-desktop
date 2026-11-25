//! whisper-cli `-oj` JSON 출력 파싱 - 타임스탬프 세그먼트와 감지 언어.

use serde::Deserialize;

use crate::error::{AiError, Result};

/// 전사 세그먼트 한 개. 시간은 오디오 시작 기준 밀리초.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    /// 앞뒤 공백을 제거한 원문 (번역·정규화하지 않는다 - 원문 언어 보존).
    pub text: String,
}

/// 전사 결과. STT 원문은 AI 생성 설명과 분리 저장한다 (db.md §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    /// whisper 가 감지한(또는 지정한) 언어 코드 (`ko`, `en`). 알 수 없으면 빈 문자열.
    pub language: String,
    pub segments: Vec<Segment>,
}

impl Transcript {
    /// 세그먼트를 공백으로 이은 전체 텍스트.
    pub fn full_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Deserialize)]
struct Raw {
    result: Option<RawResult>,
    #[serde(default)]
    transcription: Vec<RawSegment>,
}

#[derive(Deserialize)]
struct RawResult {
    #[serde(default)]
    language: String,
}

#[derive(Deserialize)]
struct RawSegment {
    offsets: RawOffsets,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct RawOffsets {
    from: i64,
    to: i64,
}

/// JSON 문서를 [`Transcript`] 로 바꾼다. 빈 텍스트 세그먼트는 버리고, 음수 시간·끝<시작은 오류로 본다.
pub fn parse_whisper_json(json: &str) -> Result<Transcript> {
    let bad = |reason: String| AiError::ToolOutput {
        tool: "whisper-cli",
        reason,
    };
    // BOM 이 붙어 오는 빌드를 허용한다.
    let raw: Raw = serde_json::from_str(json.trim_start_matches('\u{feff}'))
        .map_err(|e| bad(e.to_string()))?;
    let mut segments = Vec::with_capacity(raw.transcription.len());
    for s in raw.transcription {
        if s.offsets.from < 0 || s.offsets.to < s.offsets.from {
            return Err(bad(format!(
                "invalid segment offsets {}..{}",
                s.offsets.from, s.offsets.to
            )));
        }
        let text = s.text.trim();
        if text.is_empty() {
            continue;
        }
        segments.push(Segment {
            start_ms: s.offsets.from as u64,
            end_ms: s.offsets.to as u64,
            text: text.to_string(),
        });
    }
    Ok(Transcript {
        language: raw.result.map(|r| r.language).unwrap_or_default(),
        segments,
    })
}
