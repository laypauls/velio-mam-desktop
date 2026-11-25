//! CLIP 이미지 인코더 (ONNX `pixel_values[B,3,224,224]` -> `image_embeds[B,512]`).

use std::path::Path;
use std::sync::Mutex;

use image::DynamicImage;
use ort::session::Session;
use ort::value::Tensor;

use super::EmbedConfig;
use super::image_pre::{CLIP_SIZE, CLIP_TENSOR_LEN, decode_image, preprocess_clip};
use super::math::l2_normalize;
use crate::error::{AiError, Result};
use crate::local::runtime::build_session;
use crate::models::{ModelPaths, ids};

pub const CLIP_IMAGE_DIM: usize = 512;
/// 벡터와 함께 저장하는 모델 식별자 (모델 교체 시 새 인덱스를 만든다).
pub const CLIP_IMAGE_MODEL_ID: &str = "clip-vit-b32-vision";

pub struct ImageEmbedder {
    session: Mutex<Session>,
    batch_size: usize,
}

impl ImageEmbedder {
    /// 다운로드된 모델(`ModelPaths`)에서 만든다.
    pub fn from_paths(paths: &ModelPaths, cfg: EmbedConfig) -> Result<Self> {
        debug_assert_eq!(paths.model_id, ids::CLIP_VISION);
        Self::load(paths.file("model.onnx")?, cfg)
    }

    pub fn load(model: &Path, cfg: EmbedConfig) -> Result<Self> {
        Ok(Self {
            session: Mutex::new(build_session(model, cfg.threads)?),
            batch_size: cfg.batch_size.max(1),
        })
    }

    /// 이미지 파일 1개 (읽기 전용으로 연다).
    pub fn embed_file(&self, path: &Path) -> Result<Vec<f32>> {
        let img = decode_image(path)?;
        Ok(self.embed_images(&[&img])?.remove(0))
    }

    /// 입력 순서대로 단위 길이 512차원 벡터를 돌려준다.
    pub fn embed_images(&self, images: &[&DynamicImage]) -> Result<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Err(AiError::InvalidInput("empty image batch".into()));
        }
        let mut out = Vec::with_capacity(images.len());
        for chunk in images.chunks(self.batch_size) {
            let mut pixels = Vec::with_capacity(chunk.len() * CLIP_TENSOR_LEN);
            for img in chunk {
                pixels.extend(preprocess_clip(img));
            }
            out.extend(self.run_pixels(chunk.len(), pixels)?);
        }
        Ok(out)
    }

    /// 전처리를 마친 `[n,3,224,224]` 버퍼를 추론한다 (측정 도구에서 전처리와 추론 시간을 나눌 때 쓴다).
    pub fn run_pixels(&self, n: usize, pixels: Vec<f32>) -> Result<Vec<Vec<f32>>> {
        debug_assert_eq!(pixels.len(), n * CLIP_TENSOR_LEN);
        let size = CLIP_SIZE as usize;
        let input = Tensor::from_array(([n, 3, size, size], pixels))?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| AiError::Onnx("session lock poisoned".into()))?;
        let outputs = session.run(ort::inputs!["pixel_values" => input])?;
        let (dims, data) = outputs["image_embeds"].try_extract_tensor::<f32>()?;
        if **dims != [n as i64, CLIP_IMAGE_DIM as i64] {
            return Err(AiError::ModelOutput(format!(
                "image_embeds shape {:?}, expected [{n}, {CLIP_IMAGE_DIM}]",
                &**dims
            )));
        }
        data.chunks_exact(CLIP_IMAGE_DIM)
            .map(|row| {
                let mut v = row.to_vec();
                l2_normalize(&mut v)?;
                Ok(v)
            })
            .collect()
    }
}
