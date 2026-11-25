//! 썸네일 생성 — 긴 변 320px / 1024px WebP, EXIF 방향 적용, `asset_id` 기준 샤딩 저장 (D5).
//!
//! 원본은 읽기 전용으로만 열고, 산출물은 호출자가 준 캐시 루트 아래에만 쓴다.

use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};

use crate::error::{MediaError, Result};
use crate::format::{Container, MediaKind, detect, is_raw_extension};
use crate::frames::representative_positions;
use crate::image_meta::read_image_meta;
use crate::raw_preview::extract_raw_preview;
use crate::tool::MediaTools;
use crate::wic::decode_with_wic;

pub const SMALL_EDGE: u32 = 320;
pub const LARGE_EDGE: u32 = 1024;
/// WebP 인코딩 품질 (0..=100).
pub const WEBP_QUALITY: f32 = 80.0;
/// 디코딩 허용 최대 픽셀 수 (RGBA8 기준 약 256MB). 초과하면 전체 디코딩 전에 거부한다.
pub const DEFAULT_MAX_PIXELS: u64 = 64_000_000;
/// `image` 디코더 메모리 할당 상한.
const MAX_ALLOC_BYTES: u64 = 768 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbSize {
    Small,
    Large,
}

impl ThumbSize {
    pub fn edge(self) -> u32 {
        match self {
            ThumbSize::Small => SMALL_EDGE,
            ThumbSize::Large => LARGE_EDGE,
        }
    }
}

/// 어떤 경로로 디코딩했는가 (호환표·진단용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeMethod {
    ImageCrate,
    RawPreview,
    FfmpegImage,
    VideoFrame,
    /// Windows Imaging Component (HEIC 의 마지막 폴백, P2-3 D7)
    Wic,
}

#[derive(Debug, Clone)]
pub struct ThumbnailOptions {
    pub max_pixels: u64,
}

impl Default for ThumbnailOptions {
    fn default() -> Self {
        Self {
            max_pixels: DEFAULT_MAX_PIXELS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThumbnailOutput {
    pub small: PathBuf,
    pub large: PathBuf,
    /// 큰 썸네일의 실제 크기.
    pub large_size: (u32, u32),
    pub method: DecodeMethod,
}

/// 샤딩된 썸네일 경로: `<cache_root>/thumbnails/<edge>/<aa>/<bb>/<asset_id>.webp`.
/// `aa`/`bb` 는 asset_id 하위 16비트의 16진수 — 연속 ID 가 폴더에 고르게 퍼져 폴더당 파일 수를 억제한다.
pub fn thumbnail_path(cache_root: &Path, asset_id: i64, size: ThumbSize) -> PathBuf {
    let id = asset_id as u64;
    cache_root
        .join("thumbnails")
        .join(size.edge().to_string())
        .join(format!("{:02x}", id & 0xFF))
        .join(format!("{:02x}", (id >> 8) & 0xFF))
        .join(format!("{asset_id}.webp"))
}

pub(crate) fn fit(img: &DynamicImage, edge: u32) -> DynamicImage {
    if img.width().max(img.height()) <= edge {
        img.clone()
    } else {
        img.thumbnail(edge, edge)
    }
}

/// 손실 WebP (품질 `WEBP_QUALITY`). image crate 의 WebP 인코더는 무손실만 지원해 사진 썸네일이 수 배 커지므로
/// libwebp 를 쓴다. 투명도가 있으면 알파 채널을 보존한다.
pub(crate) fn encode_webp(img: &DynamicImage) -> Result<Vec<u8>> {
    let (w, h) = (img.width(), img.height());
    let mem = if img.color().has_alpha() {
        webp::Encoder::from_rgba(img.to_rgba8().as_raw(), w, h).encode(WEBP_QUALITY)
    } else {
        webp::Encoder::from_rgb(img.to_rgb8().as_raw(), w, h).encode(WEBP_QUALITY)
    };
    if mem.is_empty() {
        return Err(MediaError::Encode("libwebp returned no data".into()));
    }
    Ok(mem.to_vec())
}

/// 임시 파일에 쓰고 rename 으로 교체한다 (같은 asset 재생성 시 덮어쓰기, 중간 상태 노출 없음).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().expect("thumbnail path has parent");
    fs::create_dir_all(dir).map_err(|e| MediaError::io(dir, e))?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| MediaError::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        MediaError::io(path, e)
    })
}

fn decode_error(e: image::ImageError) -> MediaError {
    MediaError::Decode(e.to_string())
}

/// `image` crate 로 디코딩한다. 크기를 먼저 확인해 상한을 넘으면 디코딩하지 않는다.
fn decode_with_image(
    reader: ImageReader<impl std::io::BufRead + std::io::Seek>,
    max_pixels: u64,
) -> Result<DynamicImage> {
    let mut decoder = reader
        .with_guessed_format()
        .map_err(|e| MediaError::Decode(e.to_string()))?
        .into_decoder()
        .map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(MediaError::ImageTooLarge {
            width,
            height,
            max_pixels,
        });
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_ALLOC_BYTES);
    decoder.set_limits(limits).map_err(decode_error)?;
    DynamicImage::from_decoder(decoder).map_err(decode_error)
}

fn decode_image_file(path: &Path, max_pixels: u64) -> Result<DynamicImage> {
    let file = File::open(path).map_err(|e| MediaError::io(path, e))?;
    decode_with_image(ImageReader::new(BufReader::new(file)), max_pixels)
}

fn decode_raw_preview(path: &Path, max_pixels: u64) -> Result<DynamicImage> {
    let jpeg = extract_raw_preview(path)?;
    decode_with_image(
        ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg),
        max_pixels,
    )
}

/// 소스를 디코딩해 (이미지, 방식, 방향 적용 필요 여부) 를 돌려준다. 이미 방향이 적용된 경로(ffmpeg)는 적용 안 함.
fn decode_source(
    tools: &MediaTools,
    path: &Path,
    opts: &ThumbnailOptions,
) -> Result<(DynamicImage, DecodeMethod, u8)> {
    let det = detect(path)?;
    match det.kind {
        Some(MediaKind::Image) => {
            let ffmpeg_image = |cause: Option<MediaError>| {
                if tools.has_ffmpeg() {
                    tools
                        .extract_frame(path, None, LARGE_EDGE)
                        .map(|i| (i, DecodeMethod::FfmpegImage, 1))
                } else {
                    Err(cause.unwrap_or(MediaError::ToolNotFound {
                        tool: "ffmpeg",
                        searched: "no decoder for this image format".into(),
                    }))
                }
            };
            let orientation = || read_image_meta(path).map(|m| m.orientation).unwrap_or(1);
            if is_raw_extension(&det.extension) {
                return match decode_raw_preview(path, opts.max_pixels) {
                    Ok(img) => Ok((img, DecodeMethod::RawPreview, orientation())),
                    Err(e) => ffmpeg_image(Some(e)),
                };
            }
            match det.container {
                // HEIC/AVIF: FFmpeg -> 실패하면 Windows WIC (HEIF 확장 설치 시). 두 사유를 함께 남긴다
                Some(Container::Heif | Container::Avif) => match ffmpeg_image(None) {
                    Ok(v) => Ok(v),
                    Err(ffmpeg_err) => match decode_with_wic(path, opts.max_pixels) {
                        Ok(img) => Ok((img, DecodeMethod::Wic, 1)),
                        Err(wic_err) => Err(MediaError::Decode(format!(
                            "ffmpeg: {ffmpeg_err}; wic fallback: {wic_err}"
                        ))),
                    },
                },
                _ => match decode_image_file(path, opts.max_pixels) {
                    Ok(img) => Ok((img, DecodeMethod::ImageCrate, orientation())),
                    // 너무 큰 이미지·image crate 미지원/손상 -> FFmpeg 로 시도 (스케일링하며 디코딩)
                    Err(e @ (MediaError::ImageTooLarge { .. } | MediaError::Decode(_))) => {
                        ffmpeg_image(Some(e))
                    }
                    Err(e) => Err(e),
                },
            }
        }
        Some(MediaKind::Video) => {
            let duration = tools.probe(path).ok().and_then(|p| p.duration_secs);
            let t = representative_positions(duration)[0];
            let img = match tools.extract_frame(path, Some(t), LARGE_EDGE) {
                // 짧거나 잘린 파일에서 탐색 지점에 프레임이 없으면 첫 프레임으로 재시도
                Err(MediaError::InvalidOutput(_)) if t > 0.0 => {
                    tools.extract_frame(path, Some(0.0), LARGE_EDGE)?
                }
                other => other?,
            };
            Ok((img, DecodeMethod::VideoFrame, 1))
        }
        Some(MediaKind::Audio) => Err(MediaError::Unsupported("audio has no thumbnail")),
        None => Err(MediaError::Unsupported("not a supported media file")),
    }
}

/// 썸네일 두 단계(320/1024)를 만들어 `cache_root` 아래에 저장한다.
pub fn create_thumbnails(
    tools: &MediaTools,
    source: &Path,
    asset_id: i64,
    cache_root: &Path,
    opts: &ThumbnailOptions,
) -> Result<ThumbnailOutput> {
    let (img, method, exif_orientation) = decode_source(tools, source, opts)?;
    // 축소한 뒤 회전해 비용을 줄인다.
    let orientation = Orientation::from_exif(exif_orientation).unwrap_or(Orientation::NoTransforms);
    let mut large = fit(&img, LARGE_EDGE);
    large.apply_orientation(orientation);
    let small = fit(&large, SMALL_EDGE);

    let (small_path, large_path) = (
        thumbnail_path(cache_root, asset_id, ThumbSize::Small),
        thumbnail_path(cache_root, asset_id, ThumbSize::Large),
    );
    write_atomic(&large_path, &encode_webp(&large)?)?;
    write_atomic(&small_path, &encode_webp(&small)?)?;
    Ok(ThumbnailOutput {
        small: small_path,
        large: large_path,
        large_size: (large.width(), large.height()),
        method,
    })
}
