//! 자산 단위 전사 작업 (P3-4 D1) - `SttEngine::transcribe` 를 감싸 저장용 결과(`TranscriptResult`)를 만든다.
//!
//! 임시 WAV·JSON 은 `work_dir` 아래 작업 폴더에만 쓰이고 성공·실패·취소와 무관하게 삭제된다(`JobDir`).
//! 전사 텍스트는 번역·정규화하지 않는다(원문 언어 보존). 실패 이력 기록은 호출자(작업 대기열)의 몫이다.

use std::path::Path;

use super::{CancelToken, Segment, SttEngine};
use crate::error::Result;

/// 전사 엔진 식별자 (저장 시 `transcripts.engine`).
pub const ENGINE_ID: &str = "whisper.cpp";

/// 저장 가능한 전사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptResult {
    /// 감지(또는 지정)된 원문 언어. 알 수 없으면 빈 문자열.
    pub language: String,
    pub segments: Vec<Segment>,
    pub engine_id: String,
    /// 모델 식별자 (모델 파일명에서 유도).
    pub model_id: String,
    /// 마지막 구간의 끝 시각(ms). 구간이 없으면 0.
    pub duration_ms: u64,
}

/// `media_path`(읽기 전용)를 전사한다. 출력은 `work_dir` 아래에만 만든다.
pub fn transcribe_asset(
    engine: &SttEngine,
    media_path: &Path,
    work_dir: &Path,
    cancel: Option<&CancelToken>,
) -> Result<TranscriptResult> {
    let t = engine.transcribe(media_path, work_dir, cancel)?;
    let duration_ms = t.segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
    Ok(TranscriptResult {
        language: t.language,
        segments: t.segments,
        engine_id: ENGINE_ID.to_string(),
        model_id: model_id(&engine.paths.model),
        duration_ms,
    })
}

/// `ggml-base.bin` -> `ggml-base`. 모델 폴더 규약(`whisper-base/model.bin`)이면 폴더명을 쓴다.
fn model_id(model: &Path) -> String {
    let stem = model.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem == "model"
        && let Some(dir) = model
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
    {
        return dir.to_string();
    }
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_id_uses_stem_or_folder() {
        assert_eq!(model_id(Path::new("m/ggml-base.bin")), "ggml-base");
        assert_eq!(
            model_id(Path::new("m/whisper-base/model.bin")),
            "whisper-base"
        );
        assert_eq!(model_id(Path::new("model.bin")), "model");
    }
}
