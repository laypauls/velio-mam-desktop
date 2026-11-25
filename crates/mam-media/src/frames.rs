//! 영상 대표 프레임·이미지 폴백 디코딩 — FFmpeg 사이드카가 PNG 를 stdout 으로 내보낸다.
//!
//! 출력은 파이프로만 받으므로 FFmpeg 가 디스크에 아무것도 쓰지 않는다 (원본 폴더 쓰기 없음).

use std::ffi::OsString;
use std::path::Path;

use image::DynamicImage;

use crate::error::{MediaError, Result};
use crate::tool::{MAX_FRAME_OUTPUT, MediaTools, Tool};

/// 대표 프레임 위치 (길이 비율) — 10%, 50%, 90% (D6). 썸네일은 첫 번째(10%)를 쓴다.
pub const REPRESENTATIVE_RATIOS: [f64; 3] = [0.10, 0.50, 0.90];

/// 길이(초)에서 대표 프레임 시각들을 계산한다. 길이가 0 이하/비정상이면 `[0.0]`.
pub fn representative_positions(duration_secs: Option<f64>) -> Vec<f64> {
    match duration_secs {
        Some(d) if d.is_finite() && d > 0.0 => {
            REPRESENTATIVE_RATIOS.iter().map(|r| d * r).collect()
        }
        _ => vec![0.0],
    }
}

impl MediaTools {
    /// 한 프레임을 긴 변 `max_edge` 이하로 축소한 PNG 로 받아 디코딩한다.
    /// `at_secs` 가 `Some` 이면 입력 탐색(`-ss`)으로 해당 시각 근처 프레임, `None` 이면 첫 프레임(이미지 입력용).
    /// 회전 메타데이터는 FFmpeg 가 자동 적용한다.
    pub fn extract_frame(
        &self,
        path: &Path,
        at_secs: Option<f64>,
        max_edge: u32,
    ) -> Result<DynamicImage> {
        let mut args: Vec<OsString> = ["-nostdin", "-v", "error", "-threads"]
            .into_iter()
            .map(Into::into)
            .collect();
        args.push(self.threads.max(1).to_string().into());
        if let Some(t) = at_secs {
            args.push("-ss".into());
            args.push(format!("{t:.3}").into());
        }
        args.push("-i".into());
        args.push(path.as_os_str().to_os_string());
        let scale = format!(
            "scale=w='min({e},iw)':h='min({e},ih)':force_original_aspect_ratio=decrease",
            e = max_edge
        );
        args.extend(
            ["-frames:v", "1", "-an", "-sn", "-vf"]
                .into_iter()
                .map(OsString::from),
        );
        args.push(scale.into());
        args.extend(
            ["-f", "image2pipe", "-c:v", "png", "pipe:1"]
                .into_iter()
                .map(OsString::from),
        );
        let out = self.run(Tool::Ffmpeg, &args, MAX_FRAME_OUTPUT)?;
        if out.is_empty() {
            return Err(MediaError::InvalidOutput("ffmpeg produced no frame".into()));
        }
        image::load_from_memory_with_format(&out, image::ImageFormat::Png)
            .map_err(|e| MediaError::Decode(format!("ffmpeg frame: {e}")))
    }
}
