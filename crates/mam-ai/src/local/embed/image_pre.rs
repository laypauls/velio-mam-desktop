//! CLIP 이미지 전처리 — 짧은 변 224 리사이즈(bicubic) -> 중앙 224 자르기 -> 0..1 -> 평균/표준편차 정규화 -> CHW.
//!
//! HuggingFace `CLIPImageProcessor`(`preprocessor_config.json`)와 같은 규칙이다. 알파 채널은 버린다(`convert("RGB")`와 동일).
//! 리사이즈 필터는 `image` 크레이트의 Catmull-Rom(bicubic)이며 PIL bicubic 과 픽셀 단위로 같지는 않다 -
//! 임베딩 코사인 차이는 측정 보고서에 기록한다.

use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, Limits};

use crate::error::{AiError, Result};

pub const CLIP_SIZE: u32 = 224;
// preprocessor_config.json 의 값을 그대로 옮긴다 (f32 로 반올림됨).
#[allow(clippy::excessive_precision)]
pub const CLIP_MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
#[allow(clippy::excessive_precision)]
pub const CLIP_STD: [f32; 3] = [0.268_629_54, 0.261_302_58, 0.275_777_11];
/// 텐서 하나의 원소 수 (3 x 224 x 224).
pub const CLIP_TENSOR_LEN: usize = 3 * (CLIP_SIZE as usize) * (CLIP_SIZE as usize);

/// 디코딩 한도: 한 변 20,000px, 할당 1GiB. 초대형 이미지를 전부 디코딩하다 메모리를 소진하지 않게 한다.
const MAX_DIM: u32 = 20_000;
const MAX_ALLOC: u64 = 1 << 30;

/// 파일을 **읽기 전용**으로 열어 디코딩한다 (EXIF 회전은 적용하지 않는다 - 호출자가 필요하면 처리).
pub fn decode_image(path: &Path) -> Result<DynamicImage> {
    let mut reader = ImageReader::open(path)
        .map_err(|e| AiError::io(path, e))?
        .with_guessed_format()
        .map_err(|e| AiError::io(path, e))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIM);
    limits.max_image_height = Some(MAX_DIM);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| AiError::Decode(format!("{}: {e}", path.display())))
}

/// 메모리 바이트열에서 디코딩한다 (같은 한도).
pub fn decode_bytes(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| AiError::Decode(e.to_string()))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIM);
    limits.max_image_height = Some(MAX_DIM);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    reader.decode().map_err(|e| AiError::Decode(e.to_string()))
}

/// 이미지 1장을 `[3, 224, 224]` (CHW) f32 로 변환한다.
pub fn preprocess_clip(img: &DynamicImage) -> Vec<f32> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    // 짧은 변이 224 가 되도록 비율 유지 (HF: int(size * long / short)).
    let (nw, nh) = if w <= h {
        (
            CLIP_SIZE,
            ((CLIP_SIZE as u64 * h as u64) / w.max(1) as u64).max(CLIP_SIZE as u64) as u32,
        )
    } else {
        (
            ((CLIP_SIZE as u64 * w as u64) / h.max(1) as u64).max(CLIP_SIZE as u64) as u32,
            CLIP_SIZE,
        )
    };
    let resized = if (nw, nh) == (w, h) {
        rgb
    } else {
        image::imageops::resize(&rgb, nw, nh, FilterType::CatmullRom)
    };
    let left = (nw - CLIP_SIZE) / 2;
    let top = (nh - CLIP_SIZE) / 2;

    let plane = (CLIP_SIZE * CLIP_SIZE) as usize;
    let mut out = vec![0f32; CLIP_TENSOR_LEN];
    for y in 0..CLIP_SIZE {
        for x in 0..CLIP_SIZE {
            let px = resized.get_pixel(left + x, top + y);
            let i = (y * CLIP_SIZE + x) as usize;
            for c in 0..3 {
                out[c * plane + i] = (px[c] as f32 / 255.0 - CLIP_MEAN[c]) / CLIP_STD[c];
            }
        }
    }
    out
}
