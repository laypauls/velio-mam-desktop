//! 외부 전송 페이로드 (PL-A3) — MAM 이 만든 파생 이미지(썸네일·대표 프레임)와 정제된 메타데이터만 담는다.
//! 원본 경로·원본 파일은 이 계층에 들어오지 못한다 (입력이 바이트 + 파일 이름뿐).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, ImageReader};

use super::error::ExternalError;
use super::types::{AnalysisKind, TransferScope};

const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    /// 오디오는 이미지 없이 메타데이터 텍스트만 보낸다 (원본 음성 외부 전송 금지).
    Audio,
}

impl MediaType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// 사용자 필드 값 하나의 최대 글자 수 / 필드 이름 최대 글자 수 / 사용자 필드 블록 전체 최대 글자 수 (글자 수 기준, 바이트 아님)
pub const MAX_USER_FIELD_VALUE_CHARS: usize = 200;
pub const MAX_USER_FIELD_NAME_CHARS: usize = 100;
pub const MAX_USER_FIELDS_TOTAL_CHARS: usize = 2_000;

/// 외부로 나가는 메타데이터: 파일 이름(디렉터리 제거)·촬영일·유형·길이, 그리고 사용자 메타데이터 필드 `(이름, 표시 값)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedMeta {
    pub file_name: String,
    pub captured_at: Option<String>,
    pub media_type: MediaType,
    pub duration_ms: Option<u64>,
    /// 값이 있는 사용자 필드(필드 표시 순서). `with_user_fields` 가 정제·자른다.
    pub user_fields: Vec<(String, String)>,
}

impl SanitizedMeta {
    /// `name_or_path` 에 경로가 섞여 있어도 마지막 구성요소만 남긴다 (`/`·`\` 모두).
    pub fn new(
        name_or_path: &str,
        captured_at: Option<String>,
        media_type: MediaType,
        duration_ms: Option<u64>,
    ) -> Self {
        let base = name_or_path.rsplit(['/', '\\']).next().unwrap_or("");
        let cleaned: String = base
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_NAME_CHARS)
            .collect();
        Self {
            file_name: if cleaned.is_empty() {
                "unknown".into()
            } else {
                cleaned
            },
            captured_at: captured_at
                .map(|s| s.chars().filter(|c| !c.is_control()).take(40).collect()),
            media_type,
            duration_ms,
            user_fields: Vec::new(),
        }
    }

    /// 사용자 필드를 정제해 담는다: 제어 문자 제거, 이름 100자·값 200자로 자름, 이름/값이 비면 제외,
    /// 전체(이름+값 글자 수 합)가 2,000자를 넘으면 필드 순서대로 채우다가 남은 한도에서 자르고 나머지는 버린다.
    pub fn with_user_fields(mut self, fields: Vec<(String, String)>) -> Self {
        let clean = |s: &str, max: usize| -> String {
            s.chars().filter(|c| !c.is_control()).take(max).collect()
        };
        let mut left = MAX_USER_FIELDS_TOTAL_CHARS;
        let mut out = Vec::new();
        for (name, value) in fields {
            let name = clean(&name, MAX_USER_FIELD_NAME_CHARS);
            let value = clean(&value, MAX_USER_FIELD_VALUE_CHARS);
            if name.is_empty() || value.is_empty() {
                continue;
            }
            let name_len = name.chars().count();
            if left <= name_len {
                break;
            }
            left -= name_len;
            let value: String = value.chars().take(left).collect();
            left -= value.chars().count();
            out.push((name, value));
        }
        self.user_fields = out;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedImage {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

/// 분석 요청 1건. `images` 는 MAM 이 만든 썸네일/대표 프레임 바이트다.
#[derive(Debug, Clone)]
pub struct AnalyzeRequest {
    pub asset_id: Option<i64>,
    /// `None` 이면 공급자 기본 모델 (`models::default_model`).
    pub model: Option<String>,
    pub language: String,
    pub kinds: Vec<AnalysisKind>,
    pub scope: TransferScope,
    pub meta: SanitizedMeta,
    pub images: Vec<Vec<u8>>,
}

impl AnalyzeRequest {
    pub(crate) fn validate(&self) -> Result<(), ExternalError> {
        let lang_ok = !self.language.is_empty()
            && self.language.len() <= 16
            && self
                .language
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !lang_ok {
            return Err(ExternalError::InvalidInput("language".into()));
        }
        if self.kinds.is_empty() {
            return Err(ExternalError::InvalidInput("no analysis kind".into()));
        }
        if self.images.is_empty() && self.meta.media_type != MediaType::Audio {
            return Err(ExternalError::InvalidInput("no image".into()));
        }
        Ok(())
    }

    /// 범위 설정에 맞게 프레임을 고르고(영상은 균등 간격으로 최대 `videoFrames` 장) JPEG 로 재인코딩한다.
    pub(crate) fn prepare_images(&self) -> Result<Vec<PreparedImage>, ExternalError> {
        let scope = self.scope.clamped();
        let limit = match self.meta.media_type {
            MediaType::Video => scope.video_frames as usize,
            _ => 1,
        };
        pick_evenly(&self.images, limit)
            .into_iter()
            .map(|b| {
                let (bytes, mime) = prepare_image(b, scope.image_max_edge)?;
                Ok(PreparedImage { bytes, mime })
            })
            .collect()
    }
}

fn pick_evenly(items: &[Vec<u8>], n: usize) -> Vec<&Vec<u8>> {
    if items.len() <= n {
        return items.iter().collect();
    }
    (0..n).map(|i| &items[i * items.len() / n]).collect()
}

/// 긴 변을 `max_edge`(256..=2048 로 보정) 이하로 줄이고 JPEG 로 다시 인코딩한다. 재인코딩으로 EXIF 등 부가 정보가 사라진다.
pub fn prepare_image(
    bytes: &[u8],
    max_edge: u32,
) -> Result<(Vec<u8>, &'static str), ExternalError> {
    let invalid = |m: &str| ExternalError::InvalidInput(m.into());
    if bytes.is_empty() || bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid("image size"));
    }
    let max_edge = max_edge.clamp(256, 2048);
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("image format"))?
        .decode()
        .map_err(|_| invalid("image decode"))?;
    let img = if img.width().max(img.height()) > max_edge {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|_| invalid("image encode"))?;
    Ok((out, "image/jpeg"))
}
