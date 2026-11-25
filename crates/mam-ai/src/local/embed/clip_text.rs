//! 다국어 CLIP 텍스트 인코더 - 이미지 임베딩과 같은 공간에서 한국어/영어 질의로 이미지를 찾는다.

use std::path::Path;
use std::sync::Mutex;

use safetensors::SafeTensors;

use super::EmbedConfig;
use super::encoder::{Projection, TextEncoder};
use super::tokenize::TextTokenizer;
use crate::error::{AiError, Result};
use crate::local::runtime::build_session;
use crate::models::{ModelPaths, ids};

pub const CLIP_TEXT_DIM: usize = 512;
pub const CLIP_TEXT_MODEL_ID: &str = "clip-b32-multilingual-text";
/// 모델의 `sentence_bert_config.json` `max_seq_length`.
const MAX_TOKENS: usize = 128;
const HIDDEN: usize = 768;

pub struct ClipTextEmbedder {
    enc: TextEncoder,
}

impl ClipTextEmbedder {
    pub fn from_paths(paths: &ModelPaths, cfg: EmbedConfig) -> Result<Self> {
        debug_assert_eq!(paths.model_id, ids::CLIP_TEXT_ML);
        Self::load(
            paths.file("model.onnx")?,
            paths.file("tokenizer.json")?,
            paths.file("dense.safetensors")?,
            cfg,
        )
    }

    pub fn load(model: &Path, tokenizer: &Path, dense: &Path, cfg: EmbedConfig) -> Result<Self> {
        let session = build_session(model, cfg.threads)?;
        let needs_token_types = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        Ok(Self {
            enc: TextEncoder {
                session: Mutex::new(session),
                tokenizer: TextTokenizer::load(tokenizer, MAX_TOKENS)?,
                needs_token_types,
                hidden: HIDDEN,
                projection: Some(load_dense(dense)?),
                batch_size: cfg.batch_size.max(1),
            },
        })
    }

    /// 입력 순서대로 단위 길이 512차원 벡터. 빈 문자열은 오류, 128 토큰을 넘으면 잘린다.
    pub fn embed_texts(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let owned: Vec<String> = texts.iter().map(|t| (*t).to_string()).collect();
        self.enc.embed(&owned)
    }

    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed_texts(&[text])?.remove(0))
    }

    pub fn dimension(&self) -> usize {
        self.enc.out_dim()
    }

    /// 토큰 상한 (시험용).
    pub fn max_tokens(&self) -> usize {
        self.enc.tokenizer.max_len
    }
}

/// `2_Dense/model.safetensors` 의 2차원 가중치(`[512, 768]`)를 읽는다. 토치 `Linear` 는 `[out, in]` 순서다.
fn load_dense(path: &Path) -> Result<Projection> {
    let bytes = std::fs::read(path).map_err(|e| AiError::io(path, e))?;
    let st = SafeTensors::deserialize(&bytes)
        .map_err(|e| AiError::ModelOutput(format!("dense safetensors: {e}")))?;
    let (_, view) = st
        .tensors()
        .into_iter()
        .find(|(_, v)| v.shape().len() == 2)
        .ok_or_else(|| AiError::ModelOutput("dense safetensors has no 2-D weight".into()))?;
    let shape = view.shape();
    if shape != [CLIP_TEXT_DIM, HIDDEN] || view.dtype() != safetensors::Dtype::F32 {
        return Err(AiError::ModelOutput(format!(
            "dense weight {:?} {:?}, expected F32 [{CLIP_TEXT_DIM}, {HIDDEN}]",
            view.dtype(),
            shape
        )));
    }
    let weight = view
        .data()
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Ok(Projection {
        weight,
        in_dim: HIDDEN,
        out_dim: CLIP_TEXT_DIM,
    })
}
