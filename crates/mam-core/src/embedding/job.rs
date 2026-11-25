//! 자산 임베딩 작업 함수 — 시각·의미·질의.

use std::path::{Component, Path, PathBuf};

use mam_vector::{EmbedStore, ModelKey, NewVector, VectorKind, VectorSource, normalized};
use rusqlite::Connection;

use super::{EmbeddingError, ModelInfo, TextEncoder, VisualEncoder};

/// e5 문서 접두어 (색인 시).
pub const PASSAGE_PREFIX: &str = "passage: ";
/// e5 질의 접두어 (검색 시).
pub const QUERY_PREFIX: &str = "query: ";
/// 의미 텍스트 상한 (문자 수, 접두어 제외). e5 최대 512 토큰 — 한글은 대략 1~2자/토큰이라 보수적으로 자른다.
/// 토크나이저 쪽 절단이 최종 방어선이다.
pub const SEMANTIC_TEXT_MAX_CHARS: usize = 1_000;

/// 임베딩할 이미지 한 장. 영상은 프레임마다 `frame_index` 를 준다.
#[derive(Clone, Debug)]
pub struct ImageSource {
    pub path: PathBuf,
    pub frame_index: Option<i64>,
}

fn key_of(kind: VectorKind, m: &ModelInfo) -> ModelKey {
    ModelKey {
        kind,
        model_id: m.model_id.clone(),
        model_version: m.model_version.clone(),
        dimension: m.dimension,
    }
}

fn check_outputs(out: &[Vec<f32>], expected: usize, dim: usize) -> Result<(), EmbeddingError> {
    if out.len() != expected {
        return Err(EmbeddingError::CountMismatch {
            expected,
            found: out.len(),
        });
    }
    if let Some(bad) = out.iter().find(|v| v.len() != dim) {
        return Err(EmbeddingError::DimensionMismatch {
            expected: dim,
            found: bad.len(),
        });
    }
    Ok(())
}

/// `path` 가 `data_root` 안에 있고 `..` 를 쓰지 않는지 (원본 폴더를 읽지 않는 가드).
fn inside_data_root(path: &Path, data_root: &Path) -> bool {
    !path.components().any(|c| c == Component::ParentDir) && path.starts_with(data_root)
}

/// 자산의 시각 벡터를 만들어 저장한다. 이미지 한 장 = 벡터 하나(영상은 프레임마다). 새 `embedding_id` 들을 돌려준다.
/// `images` 의 파일은 `data_root`(MAMData) 안의 썸네일·프레임이어야 한다 — 아니면 인코더를 부르기 전에 거부한다.
/// 인코더·검증 실패 시 기존 벡터는 그대로다.
pub fn embed_visual(
    conn: &mut Connection,
    store: &EmbedStore,
    encoder: &dyn VisualEncoder,
    data_root: &Path,
    asset_id: i64,
    images: &[ImageSource],
    now: i64,
) -> Result<Vec<u64>, EmbeddingError> {
    if images.is_empty() {
        return Err(EmbeddingError::InvalidInput("no images".into()));
    }
    if let Some(bad) = images
        .iter()
        .find(|i| !inside_data_root(&i.path, data_root))
    {
        return Err(EmbeddingError::OutsideDataDir(bad.path.clone()));
    }
    let model = encoder.model();
    let paths: Vec<PathBuf> = images.iter().map(|i| i.path.clone()).collect();
    let out = encoder
        .encode_images(&paths)
        .map_err(EmbeddingError::Encoder)?;
    check_outputs(&out, images.len(), model.dimension)?;
    let vectors: Vec<NewVector<'_>> = images
        .iter()
        .zip(&out)
        .map(|(img, v)| NewVector {
            segment_id: None,
            source: if img.frame_index.is_some() {
                VectorSource::Frame
            } else {
                VectorSource::Image
            },
            frame_index: img.frame_index,
            vector: v,
        })
        .collect();
    let key = key_of(VectorKind::Visual, &model);
    Ok(store.put(conn, &key, asset_id, &vectors, now)?)
}

/// 자산의 의미 벡터를 만들어 저장한다. `text_parts` = AI 설명·키워드(설치 언어)·STT 전사 등 (빈 항목은 버린다).
/// 한 자산당 벡터 하나(`passage:` 접두어, [`SEMANTIC_TEXT_MAX_CHARS`] 에서 절단).
/// 쓸 텍스트가 없으면 기존 의미 벡터만 폐기하고 `None` 을 돌려준다.
pub fn embed_semantic(
    conn: &mut Connection,
    store: &EmbedStore,
    encoder: &dyn TextEncoder,
    asset_id: i64,
    text_parts: &[&str],
    now: i64,
) -> Result<Option<u64>, EmbeddingError> {
    let model = encoder.model();
    let key = key_of(VectorKind::Semantic, &model);
    let joined = text_parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        store.put(conn, &key, asset_id, &[], now)?;
        return Ok(None);
    }
    let text: String = joined.chars().take(SEMANTIC_TEXT_MAX_CHARS).collect();
    let out = encoder
        .encode_texts(&[format!("{PASSAGE_PREFIX}{text}")])
        .map_err(EmbeddingError::Encoder)?;
    check_outputs(&out, 1, model.dimension)?;
    let vector = NewVector {
        segment_id: None,
        source: VectorSource::Text,
        frame_index: None,
        vector: &out[0],
    };
    Ok(store
        .put(conn, &key, asset_id, &[vector], now)?
        .first()
        .copied())
}

/// 질의 벡터 (정규화됨). 의미 검색(`Semantic`)은 `query:` 접두어를 붙이고, 시각 질의(CLIP 텍스트 타워)는 그대로 보낸다.
pub fn embed_query(
    encoder: &dyn TextEncoder,
    kind: VectorKind,
    text: &str,
) -> Result<Vec<f32>, EmbeddingError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(EmbeddingError::InvalidInput("empty query".into()));
    }
    let model = encoder.model();
    let input = match kind {
        VectorKind::Semantic => format!("{QUERY_PREFIX}{text}"),
        VectorKind::Visual => text.to_string(),
    };
    let out = encoder
        .encode_texts(&[input])
        .map_err(EmbeddingError::Encoder)?;
    check_outputs(&out, 1, model.dimension)?;
    let v = &out[0];
    if v.iter().any(|x| !x.is_finite()) || v.iter().all(|x| *x == 0.0) {
        return Err(EmbeddingError::Encoder(
            "encoder returned a non-finite or zero vector".into(),
        ));
    }
    Ok(normalized(v))
}
