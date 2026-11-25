//! `multilingual-e5-small` 텍스트 의미 임베딩 (AI 설명·STT 텍스트 검색).
//!
//! E5 는 입력에 용도 접두어가 필수다 - 질의는 `query: `, 문서(설명·STT 세그먼트)는 `passage: `. 호출자가 접두어를
//! 신경 쓰지 않도록 [`TextRole`] 로 받아 내부에서 붙인다.

use std::path::Path;
use std::sync::Mutex;

use super::EmbedConfig;
use super::encoder::TextEncoder;
use super::tokenize::TextTokenizer;
use crate::error::Result;
use crate::local::runtime::build_session;
use crate::models::{ModelPaths, ids};

pub const E5_DIM: usize = 384;
pub const E5_MODEL_ID: &str = "multilingual-e5-small";
/// 모델 최대 위치 임베딩 512 (접두어 포함).
const MAX_TOKENS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// 검색어 - `query: ` 접두어.
    Query,
    /// 색인 대상 텍스트 - `passage: ` 접두어.
    Passage,
}

impl TextRole {
    fn prefix(self) -> &'static str {
        match self {
            TextRole::Query => "query: ",
            TextRole::Passage => "passage: ",
        }
    }
}

pub struct E5Embedder {
    enc: TextEncoder,
}

impl E5Embedder {
    pub fn from_paths(paths: &ModelPaths, cfg: EmbedConfig) -> Result<Self> {
        debug_assert_eq!(paths.model_id, ids::E5_SMALL);
        Self::load(
            paths.file("model.onnx")?,
            paths.file("tokenizer.json")?,
            cfg,
        )
    }

    pub fn load(model: &Path, tokenizer: &Path, cfg: EmbedConfig) -> Result<Self> {
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
                hidden: E5_DIM,
                projection: None,
                batch_size: cfg.batch_size.max(1),
            },
        })
    }

    /// 입력 순서대로 단위 길이 384차원 벡터. 빈 문자열은 오류, 512 토큰을 넘으면 잘린다.
    pub fn embed(&self, role: TextRole, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // 빈 입력 검증은 접두어를 붙이기 전에 해야 한다("query: " 만 남은 입력이 통과하지 않게).
        super::tokenize::validate_texts(texts)?;
        let prefixed: Vec<String> = texts
            .iter()
            .map(|t| format!("{}{}", role.prefix(), t.trim()))
            .collect();
        self.enc.embed(&prefixed)
    }

    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(TextRole::Query, &[text])?.remove(0))
    }

    pub fn embed_passage(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(TextRole::Passage, &[text])?.remove(0))
    }
}
