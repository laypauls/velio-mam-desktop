//! 로컬 임베딩 (D1~D3).
//!
//! - [`ImageEmbedder`] : CLIP ViT-B/32 이미지 인코더 -> 512차원
//! - [`ClipTextEmbedder`] : `clip-ViT-B-32-multilingual-v1` 텍스트 인코더(DistilBERT + 평균 풀링 + Dense) -> **같은 512차원 공간**
//! - [`E5Embedder`] : `multilingual-e5-small` 텍스트 의미 임베딩 -> 384차원 (`query:`/`passage:` 접두어)
//!
//! 모든 출력은 L2 정규화된 f32 벡터이며 거리 함수는 코사인(내적)이다. 모델은 모델 ID·버전·차원과 함께 저장해야 한다
//! (`.claude/rules/db.md` §2) - 상수 `*_MODEL_ID` 를 쓴다.

mod clip_text;
mod e5;
mod encoder;
mod image_embed;
pub mod image_pre;
mod math;
mod tokenize;

pub use clip_text::{CLIP_TEXT_DIM, CLIP_TEXT_MODEL_ID, ClipTextEmbedder};
pub use e5::{E5_DIM, E5_MODEL_ID, E5Embedder, TextRole};
pub use image_embed::{CLIP_IMAGE_DIM, CLIP_IMAGE_MODEL_ID, ImageEmbedder};

/// 임베딩 실행 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbedConfig {
    /// ONNX 연산 내부 스레드 수.
    pub threads: usize,
    /// 한 번에 추론하는 입력 수 (이미지 / 텍스트).
    pub batch_size: usize,
}

impl Default for EmbedConfig {
    /// 보수적 기본값 - 저사양 PC 에서도 UI 를 막지 않는다. 사양별 값은 [`crate::resources::ResourceConfig`].
    fn default() -> Self {
        Self {
            threads: 2,
            batch_size: 8,
        }
    }
}
