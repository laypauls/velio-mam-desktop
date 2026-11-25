//! 토크나이저 래퍼 — 절단(상한), 배치 패딩.

use std::path::Path;

use tokenizers::{Tokenizer, TruncationParams};

use crate::error::{AiError, Result};

/// 배치 하나를 모델 입력으로 만든 결과 (행 우선 `[batch, seq]`).
pub(crate) struct PaddedBatch {
    pub batch: usize,
    pub seq: usize,
    pub ids: Vec<i64>,
    pub mask: Vec<i64>,
}

pub(crate) struct TextTokenizer {
    tok: Tokenizer,
    pad_id: i64,
    pub max_len: usize,
}

impl TextTokenizer {
    /// `tokenizer.json` 을 읽고 `max_len` 토큰(특수 토큰 포함)으로 절단하도록 설정한다.
    pub fn load(path: &Path, max_len: usize) -> Result<Self> {
        let mut tok = Tokenizer::from_file(path).map_err(|e| AiError::Tokenizer(e.to_string()))?;
        tok.with_truncation(Some(TruncationParams {
            max_length: max_len,
            ..Default::default()
        }))
        .map_err(|e| AiError::Tokenizer(e.to_string()))?;
        tok.with_padding(None); // 패딩은 배치 최장 길이로 직접 한다.
        let pad_id = ["[PAD]", "<pad>"]
            .iter()
            .find_map(|t| tok.token_to_id(t))
            .ok_or_else(|| AiError::Tokenizer("no pad token in tokenizer.json".into()))?;
        Ok(Self {
            tok,
            pad_id: pad_id as i64,
            max_len,
        })
    }

    /// 텍스트마다 토큰 ID 를 구한다 (상한 초과분은 잘린다).
    pub fn encode(&self, texts: &[String]) -> Result<Vec<Vec<u32>>> {
        let enc = self
            .tok
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| AiError::Tokenizer(e.to_string()))?;
        Ok(enc.into_iter().map(|e| e.get_ids().to_vec()).collect())
    }

    /// 배치 최장 길이로 오른쪽을 패딩한다.
    pub fn pad(&self, rows: &[&[u32]]) -> PaddedBatch {
        let seq = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut ids = vec![self.pad_id; rows.len() * seq];
        let mut mask = vec![0i64; rows.len() * seq];
        for (b, row) in rows.iter().enumerate() {
            for (i, id) in row.iter().enumerate() {
                ids[b * seq + i] = *id as i64;
                mask[b * seq + i] = 1;
            }
        }
        PaddedBatch {
            batch: rows.len(),
            seq,
            ids,
            mask,
        }
    }
}

/// 빈 문자열·공백뿐인 문자열은 임베딩하지 않는다 (의미 없는 벡터가 인덱스를 오염시킨다).
pub(crate) fn validate_texts(texts: &[&str]) -> Result<()> {
    if texts.is_empty() {
        return Err(AiError::InvalidInput("empty text batch".into()));
    }
    if texts.iter().any(|t| t.trim().is_empty()) {
        return Err(AiError::InvalidInput("text must not be empty".into()));
    }
    Ok(())
}

/// 길이 순서로 정렬한 배치 인덱스 묶음 (패딩 낭비를 줄인다). 결과 복원은 호출자가 인덱스로 한다.
pub(crate) fn length_sorted_chunks(lens: &[usize], batch: usize) -> Vec<Vec<usize>> {
    let mut idx: Vec<usize> = (0..lens.len()).collect();
    idx.sort_by_key(|&i| lens[i]);
    idx.chunks(batch.max(1)).map(<[usize]>::to_vec).collect()
}
