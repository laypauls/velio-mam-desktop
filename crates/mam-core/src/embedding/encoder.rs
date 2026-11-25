//! 인코더 주입 경계 — 모델 추론은 이 trait 뒤에 있다.

use std::path::PathBuf;

/// 벡터를 만드는 모델의 정체. 저장소의 모델 키가 된다 (서로 다른 모델 벡터를 섞지 않는다).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub model_id: String,
    pub model_version: String,
    pub dimension: usize,
}

/// 이미지 -> 벡터 (CLIP 이미지 타워). 입력 순서대로 벡터를 돌려준다.
pub trait VisualEncoder {
    fn model(&self) -> ModelInfo;
    fn encode_images(&self, paths: &[PathBuf]) -> Result<Vec<Vec<f32>>, String>;
}

/// 텍스트 -> 벡터. 의미 검색은 e5 (`passage:`/`query:` 접두어는 호출 함수가 붙인다),
/// 시각 질의는 CLIP 텍스트 타워 (접두어 없음) 를 주입한다.
pub trait TextEncoder {
    fn model(&self) -> ModelInfo;
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
}
