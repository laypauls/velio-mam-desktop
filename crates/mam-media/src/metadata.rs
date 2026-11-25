//! 자산 1개의 메타데이터를 한 번에 모은다 (P2-3 메타 작업용) — 이미지는 헤더·EXIF, 영상·음성은 FFprobe.
//!
//! 원본은 읽기 전용으로만 열린다. 값은 표시용(방향·회전 적용 후 크기)으로 정리해 돌려준다.

use std::path::Path;

use crate::error::Result;
use crate::format::{Container, MediaKind, detect};
use crate::image_meta::read_image_meta;
use crate::playback::{Playback, image_in_webview, playback_of};
use crate::tool::MediaTools;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaMetadata {
    /// 방향·회전을 적용한 표시 크기
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<i64>,
    /// 이미지는 컨테이너 계열(jpeg/png/...), 영상은 영상 코덱, 음성은 음성 코덱
    pub codec: Option<String>,
    pub audio_codec: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub camera: Option<String>,
    /// EXIF 방향 1..=8 (이미지)
    pub orientation: Option<u8>,
    /// 촬영·생성 시각 (UTC Unix ms). EXIF 는 시간대가 없어 표기 그대로를 UTC 로 취급한다
    pub captured_at_ms: Option<i64>,
    pub playback: Playback,
    /// ffprobe `format_name` (영상·음성)
    pub container: Option<String>,
}

/// 메타데이터를 추출한다. 영상·음성은 ffprobe 가 없으면 `ToolNotFound`.
pub fn extract_metadata(tools: &MediaTools, path: &Path, kind: MediaKind) -> Result<MediaMetadata> {
    match kind {
        MediaKind::Image => image_metadata(tools, path),
        MediaKind::Video | MediaKind::Audio => av_metadata(tools, path, kind),
    }
}

fn container_name(c: Container) -> String {
    format!("{c:?}").to_ascii_lowercase()
}

fn image_metadata(tools: &MediaTools, path: &Path) -> Result<MediaMetadata> {
    let det = detect(path)?;
    let meta = read_image_meta(path)?;
    let (mut width, mut height) = (meta.width, meta.height);
    let mut orientation = meta.orientation;
    // HEIC/RAW 등 image crate 가 크기를 모르는 형식은 ffprobe 로 보충한다 (없으면 크기 없이 진행)
    if (width.is_none() || height.is_none())
        && tools.has_ffprobe()
        && let Ok(p) = tools.probe(path)
        && let Some(v) = p.video
    {
        let (w, h) = v.display_size();
        (width, height) = (Some(w), Some(h));
        orientation = 1; // ffprobe 크기는 회전이 이미 적용된 표시 크기다
    }
    if (5..=8).contains(&orientation) {
        std::mem::swap(&mut width, &mut height);
    }
    let camera = match (meta.camera_make, meta.camera_model) {
        (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (make, model) => make.or(model),
    };
    Ok(MediaMetadata {
        width,
        height,
        duration_ms: None,
        codec: det.container.map(container_name).or({
            if det.extension.is_empty() {
                None
            } else {
                Some(det.extension.clone())
            }
        }),
        audio_codec: None,
        fps: None,
        bitrate: None,
        camera,
        orientation: Some(orientation),
        captured_at_ms: meta.taken_at.as_deref().and_then(parse_iso_ms),
        playback: image_in_webview(&det.extension),
        container: None,
    })
}

fn av_metadata(tools: &MediaTools, path: &Path, kind: MediaKind) -> Result<MediaMetadata> {
    let p = tools.probe(path)?;
    let video = if kind == MediaKind::Video {
        p.video.as_ref()
    } else {
        None
    };
    let (width, height) = video.map_or((None, None), |v| {
        let (w, h) = v.display_size();
        (Some(w), Some(h))
    });
    let codec = match kind {
        MediaKind::Audio => p.audio.as_ref().map(|a| a.codec.clone()),
        _ => video.map(|v| v.codec.clone()),
    };
    Ok(MediaMetadata {
        width,
        height,
        duration_ms: p.duration_secs.map(|d| (d * 1000.0).round() as i64),
        codec,
        audio_codec: if kind == MediaKind::Video {
            p.audio.as_ref().map(|a| a.codec.clone())
        } else {
            None
        },
        fps: video.and_then(|v| v.fps),
        bitrate: p.bit_rate.and_then(|b| i64::try_from(b).ok()),
        camera: None,
        orientation: None,
        captured_at_ms: p.creation_time.as_deref().and_then(parse_iso_ms),
        playback: playback_of(&p).playback,
        container: (!p.container.is_empty()).then(|| p.container.clone()),
    })
}

/// 1970-01-01 기준 일수 (Howard Hinnant 의 civil_from_days 의 역).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DDTHH:MM:SS[.fff][Z|±HH:MM]` -> UTC Unix ms. 시간대 표기가 없으면 UTC 로 본다.
/// 형식이 다르거나 1900년 이전이면 `None`.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let part = std::str::from_utf8(&b[r]).ok()?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if y < 1900
        || !(1..=12).contains(&mo)
        || !(1..=31).contains(&d)
        || h > 23
        || mi > 59
        || sec > 60
    {
        return None;
    }
    let mut rest = &b[19..];
    let mut millis = 0i64;
    if rest.first() == Some(&b'.') {
        let digits: Vec<u8> = rest[1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .copied()
            .collect();
        let frac = digits
            .iter()
            .take(3)
            .fold(0i64, |a, c| a * 10 + i64::from(c - b'0'));
        millis = frac * 10i64.pow(3 - digits.len().min(3) as u32);
        rest = &rest[1 + digits.len()..];
    }
    let offset_min = match rest.first() {
        None | Some(b'Z') => 0,
        Some(sign @ (b'+' | b'-')) if rest.len() >= 6 && rest[3] == b':' => {
            let o = |i: usize| -> Option<i64> {
                let p = std::str::from_utf8(&rest[i..i + 2]).ok()?;
                p.parse().ok()
            };
            let total = o(1)? * 60 + o(4)?;
            if *sign == b'-' { -total } else { total }
        }
        Some(_) => return None,
    };
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1000 + millis)
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Validation
    use super::parse_iso_ms;

    #[test]
    fn t_proc_time_01_parses_utc_offsets_and_fractions() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00"), Some(0));
        assert_eq!(
            parse_iso_ms("2024-05-01T12:34:56Z"),
            Some(1_714_566_896_000)
        );
        assert_eq!(
            parse_iso_ms("2024-05-01T12:34:56.789000Z"),
            Some(1_714_566_896_789)
        );
        // +09:00 은 UTC 보다 9시간 빠르다
        assert_eq!(
            parse_iso_ms("2024-05-01T21:34:56+09:00"),
            Some(1_714_566_896_000)
        );
        assert_eq!(parse_iso_ms("2000-02-29T00:00:00"), Some(951_782_400_000));
    }

    #[test]
    fn t_proc_time_02_rejects_garbage_and_out_of_range() {
        for bad in [
            "",
            "garbage",
            "2024-13-01T00:00:00",
            "2024-05-01T25:00:00",
            "1800-01-01T00:00:00",
            "2024-05-01T12:34:56*",
        ] {
            assert_eq!(parse_iso_ms(bad), None, "{bad}");
        }
    }
}
