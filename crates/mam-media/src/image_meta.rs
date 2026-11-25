//! 이미지 메타데이터 — 크기(헤더만)와 EXIF(촬영일·방향·카메라).

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use exif::{In, Tag, Value};

use crate::error::{MediaError, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageMeta {
    /// 헤더에서 읽은 크기 (image crate 가 모르는 형식 - HEIC/RAW 등 - 이면 `None`; EXIF 값으로 보충).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// EXIF 방향 1..=8 (없으면 1).
    pub orientation: u8,
    /// 촬영일 `YYYY-MM-DDTHH:MM:SS` (EXIF 는 시간대가 없으므로 로컬 시각 그대로).
    pub taken_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
}

fn ascii(f: &exif::Field) -> Option<String> {
    match &f.value {
        Value::Ascii(v) => v.first().map(|b| {
            String::from_utf8_lossy(b)
                .trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_string()
        }),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

/// `2024:05:01 12:34:56` -> `2024-05-01T12:34:56`. 형식이 다르면 `None`.
fn parse_exif_datetime(s: &str) -> Option<String> {
    let b = s.as_bytes();
    if b.len() != 19
        || b[4] != b':'
        || b[7] != b':'
        || b[10] != b' '
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(0..4)
        && digits(5..7)
        && digits(8..10)
        && digits(11..13)
        && digits(14..16)
        && digits(17..19))
    {
        return None;
    }
    // 0000:00:00 같은 "값 없음" 표기 제외
    if s.starts_with("0000") {
        return None;
    }
    Some(format!(
        "{}-{}-{}T{}",
        &s[0..4],
        &s[5..7],
        &s[8..10],
        &s[11..]
    ))
}

/// 파일을 읽기 전용으로 열어 메타데이터를 읽는다. EXIF 가 없거나 깨져도 실패하지 않는다
/// (파일을 열 수 없을 때만 `Io` 오류).
pub fn read_image_meta(path: &Path) -> Result<ImageMeta> {
    let file = File::open(path).map_err(|e| MediaError::io(path, e))?;
    let mut meta = ImageMeta {
        orientation: 1,
        ..Default::default()
    };

    if let Ok(reader) = image::ImageReader::new(BufReader::new(file)).with_guessed_format()
        && let Ok((w, h)) = reader.into_dimensions()
    {
        meta.width = Some(w);
        meta.height = Some(h);
    }

    let file = File::open(path).map_err(|e| MediaError::io(path, e))?;
    let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file)) else {
        return Ok(meta);
    };
    let get = |tag| exif.get_field(tag, In::PRIMARY);
    if let Some(o) = get(Tag::Orientation).and_then(|f| f.value.get_uint(0))
        && (1..=8).contains(&o)
    {
        meta.orientation = o as u8;
    }
    meta.taken_at = [Tag::DateTimeOriginal, Tag::DateTimeDigitized, Tag::DateTime]
        .into_iter()
        .filter_map(get)
        .filter_map(ascii)
        .find_map(|s| parse_exif_datetime(&s));
    meta.camera_make = get(Tag::Make).and_then(ascii);
    meta.camera_model = get(Tag::Model).and_then(ascii);
    if meta.width.is_none() {
        meta.width = get(Tag::PixelXDimension).and_then(|f| f.value.get_uint(0));
        meta.height = get(Tag::PixelYDimension).and_then(|f| f.value.get_uint(0));
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::parse_exif_datetime;

    #[test]
    fn datetime_parsing() {
        assert_eq!(
            parse_exif_datetime("2024:05:01 12:34:56").as_deref(),
            Some("2024-05-01T12:34:56")
        );
        assert_eq!(parse_exif_datetime("0000:00:00 00:00:00"), None);
        assert_eq!(parse_exif_datetime("garbage"), None);
        assert_eq!(parse_exif_datetime("2024-05-01 12:34:56"), None);
    }
}
