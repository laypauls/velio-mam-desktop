//! 영상 미리보기 프레임 스트립 (P2-3 D4) — 대표 프레임 5장(10~90%)을 가로로 이어 붙인 WebP 한 장.
//!
//! 마우스오버 미리보기용이다. 모든 칸의 크기가 같으므로 UI 는 `폭 / 5` 로 칸을 잘라 순서대로 보여 주면 된다.
//! 동영상 트랜스코딩 프록시는 만들지 않는다 (P1-5 D7). 산출물은 호출자가 준 캐시 루트(`previews/`)에만 쓴다.

use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbaImage, imageops};

use crate::error::{MediaError, Result};
use crate::thumbnail::{encode_webp, fit, write_atomic};
use crate::tool::MediaTools;

/// 스트립에 담는 프레임 수.
pub const STRIP_FRAMES: usize = 5;
/// 한 칸의 긴 변 상한(px). 5칸을 이어도 WebP 최대 변(16383)을 넘지 않는다.
pub const STRIP_CELL_EDGE: u32 = 240;
/// 프레임 위치 (길이 비율).
const RATIOS: [f64; STRIP_FRAMES] = [0.10, 0.30, 0.50, 0.70, 0.90];

#[derive(Debug, Clone)]
pub struct StripOutput {
    pub path: PathBuf,
    /// 한 칸의 크기. 스트립 전체 크기는 `(cell.0 * STRIP_FRAMES, cell.1)`.
    pub cell: (u32, u32),
}

/// `<cache_root>/previews/<aa>/<bb>/<asset_id>_strip.webp` — 썸네일과 같은 방식으로 샤딩한다.
pub fn strip_path(cache_root: &Path, asset_id: i64) -> PathBuf {
    let id = asset_id as u64;
    cache_root
        .join("previews")
        .join(format!("{:02x}", id & 0xFF))
        .join(format!("{:02x}", (id >> 8) & 0xFF))
        .join(format!("{asset_id}_strip.webp"))
}

/// 프레임 스트립을 만들어 저장한다. 길이를 모르면(`None`/0 이하) 위치를 정할 수 없어 `Unsupported`.
/// 일부 프레임만 실패하면 직전에 얻은 프레임으로 채운다 (첫 프레임이 실패하면 오류).
pub fn create_frame_strip(
    tools: &MediaTools,
    source: &Path,
    asset_id: i64,
    cache_root: &Path,
    duration_secs: Option<f64>,
) -> Result<StripOutput> {
    let duration = duration_secs
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or(MediaError::Unsupported("unknown duration"))?;

    let mut frames: Vec<DynamicImage> = Vec::with_capacity(STRIP_FRAMES);
    let mut first_error: Option<MediaError> = None;
    for ratio in RATIOS {
        match tools.extract_frame(source, Some(duration * ratio), STRIP_CELL_EDGE) {
            Ok(img) => frames.push(fit(&img, STRIP_CELL_EDGE)),
            // 도구 자체가 없거나 시간이 초과되면 나머지도 같은 결과다 — 바로 중단
            Err(
                e @ (MediaError::ToolNotFound { .. }
                | MediaError::Timeout { .. }
                | MediaError::Io { .. }),
            ) => return Err(e),
            Err(e) => {
                first_error.get_or_insert(e);
                match frames.last().cloned() {
                    Some(prev) => frames.push(prev),
                    // 앞 칸이 없으면 채울 수 없다 — 뒤 칸이 성공하면 앞으로 채운다
                    None => frames.push(DynamicImage::new_rgba8(0, 0)),
                }
            }
        }
    }
    let Some(template) = frames.iter().find(|f| f.width() > 0).cloned() else {
        return Err(first_error.unwrap_or_else(|| MediaError::InvalidOutput("no frames".into())));
    };
    let (cw, ch) = (template.width(), template.height());
    let mut canvas = RgbaImage::new(cw * STRIP_FRAMES as u32, ch);
    let mut last_good = template;
    for (i, frame) in frames.into_iter().enumerate() {
        let frame = if frame.width() == 0 {
            last_good.clone()
        } else {
            frame
        };
        let cell = if (frame.width(), frame.height()) == (cw, ch) {
            frame.to_rgba8()
        } else {
            imageops::resize(&frame.to_rgba8(), cw, ch, imageops::FilterType::Triangle)
        };
        imageops::replace(&mut canvas, &cell, i64::from(cw) * i as i64, 0);
        last_good = frame;
    }
    let path = strip_path(cache_root, asset_id);
    let rgb = DynamicImage::ImageRgba8(canvas).to_rgb8();
    write_atomic(&path, &encode_webp(&DynamicImage::ImageRgb8(rgb))?)?;
    Ok(StripOutput {
        path,
        cell: (cw, ch),
    })
}
