//! 텍스트 트랜스포머 공통 실행 — 토큰화 -> 배치 추론 -> 평균 풀링 -> (선택 투영) -> L2 정규화.

use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;

use super::math::{l2_normalize, mean_pool};
use super::tokenize::{TextTokenizer, length_sorted_chunks, validate_texts};
use crate::error::{AiError, Result};

/// 풀링 뒤에 적용하는 선형 투영 (sentence-transformers `Dense`, 활성화 없음, bias 없음).
pub(crate) struct Projection {
    /// `[out_dim, in_dim]` 행 우선.
    pub weight: Vec<f32>,
    pub in_dim: usize,
    pub out_dim: usize,
}

impl Projection {
    fn apply(&self, x: &[f32]) -> Vec<f32> {
        self.weight
            .chunks_exact(self.in_dim)
            .map(|row| row.iter().zip(x).map(|(w, v)| w * v).sum())
            .collect()
    }
}

pub(crate) struct TextEncoder {
    pub session: Mutex<Session>,
    pub tokenizer: TextTokenizer,
    /// 모델 입력에 `token_type_ids` 가 있으면 0 으로 채운다.
    pub needs_token_types: bool,
    pub hidden: usize,
    pub projection: Option<Projection>,
    pub batch_size: usize,
}

impl TextEncoder {
    pub fn out_dim(&self) -> usize {
        self.projection.as_ref().map_or(self.hidden, |p| p.out_dim)
    }

    /// 입력 순서대로 단위 길이 벡터를 돌려준다.
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        validate_texts(&refs)?;
        let tokens = self.tokenizer.encode(texts)?;
        let lens: Vec<usize> = tokens.iter().map(Vec::len).collect();
        let mut out: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];

        for chunk in length_sorted_chunks(&lens, self.batch_size) {
            let rows: Vec<&[u32]> = chunk.iter().map(|&i| tokens[i].as_slice()).collect();
            let padded = self.tokenizer.pad(&rows);
            let shape = [padded.batch, padded.seq];

            let mut inputs = ort::inputs![
                "input_ids" => Tensor::from_array((shape, padded.ids.clone()))?,
                "attention_mask" => Tensor::from_array((shape, padded.mask.clone()))?,
            ];
            if self.needs_token_types {
                inputs.push((
                    "token_type_ids".into(),
                    Tensor::from_array((shape, vec![0i64; padded.batch * padded.seq]))?.into(),
                ));
            }
            let mut session = self
                .session
                .lock()
                .map_err(|_| AiError::Onnx("session lock poisoned".into()))?;
            let outputs = session.run(inputs)?;
            let (dims, data) = outputs["last_hidden_state"].try_extract_tensor::<f32>()?;
            let expect = [padded.batch as i64, padded.seq as i64, self.hidden as i64];
            if **dims != expect {
                return Err(AiError::ModelOutput(format!(
                    "last_hidden_state shape {:?}, expected {:?}",
                    &**dims, expect
                )));
            }
            let per_row = padded.seq * self.hidden;
            for (b, &orig) in chunk.iter().enumerate() {
                let hidden = &data[b * per_row..(b + 1) * per_row];
                let mask = &padded.mask[b * padded.seq..(b + 1) * padded.seq];
                let mut v = mean_pool(hidden, mask, self.hidden);
                if let Some(p) = &self.projection {
                    v = p.apply(&v);
                }
                l2_normalize(&mut v)?;
                out[orig] = v;
            }
        }
        Ok(out)
    }
}
