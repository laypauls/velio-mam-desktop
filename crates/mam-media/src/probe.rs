//! FFprobe 래퍼 — JSON 출력 파싱으로 길이·해상도·코덱·회전·생성일을 얻는다.

use std::ffi::OsString;
use std::path::Path;

use serde::Deserialize;

use crate::error::{MediaError, Result};
use crate::tool::{MAX_PROBE_OUTPUT, MediaTools, Tool};

#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// 시계 반대 방향 회전각 (0/90/180/270). 90/270 이면 표시 크기는 가로·세로가 바뀐다.
    pub rotation: u32,
    pub fps: Option<f64>,
}

impl VideoInfo {
    /// 회전을 적용한 표시 크기.
    pub fn display_size(&self) -> (u32, u32) {
        if self.rotation == 90 || self.rotation == 270 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

/// ffprobe 결과 요약.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaProbe {
    /// ffprobe `format_name` (예: `mov,mp4,m4a,3gp,3g2,mj2`, `matroska,webm`).
    pub container: String,
    pub duration_secs: Option<f64>,
    pub size_bytes: Option<u64>,
    pub bit_rate: Option<u64>,
    /// 컨테이너 `creation_time` 태그 (ISO 8601 문자열 그대로).
    pub creation_time: Option<String>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
    size: Option<String>,
    bit_rate: Option<String>,
    tags: Option<RawTags>,
}

#[derive(Deserialize, Default)]
struct RawTags {
    creation_time: Option<String>,
    rotate: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawDisposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct RawSideData {
    rotation: Option<f64>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    duration: Option<String>,
    tags: Option<RawTags>,
    #[serde(default)]
    disposition: RawDisposition,
    #[serde(default)]
    side_data_list: Vec<RawSideData>,
}

fn num<T: std::str::FromStr>(s: &Option<String>) -> Option<T> {
    s.as_deref().and_then(|v| v.trim().parse().ok())
}

fn secs(s: &Option<String>) -> Option<f64> {
    num::<f64>(s).filter(|d| d.is_finite() && *d >= 0.0)
}

fn fps(s: &Option<String>) -> Option<f64> {
    let (n, d) = s.as_deref()?.split_once('/')?;
    let (n, d) = (n.parse::<f64>().ok()?, d.parse::<f64>().ok()?);
    (d > 0.0 && n > 0.0).then(|| n / d)
}

fn normalize_rotation(deg: f64) -> u32 {
    (((deg.round() as i64) % 360 + 360) % 360) as u32
}

/// ffprobe `-print_format json` 출력을 파싱한다.
pub fn parse_probe_json(json: &[u8]) -> Result<MediaProbe> {
    let raw: Raw = serde_json::from_slice(json)
        .map_err(|e| MediaError::InvalidOutput(format!("json: {e}")))?;
    let format = raw
        .format
        .ok_or_else(|| MediaError::InvalidOutput("missing format".into()))?;
    if raw.streams.is_empty() {
        return Err(MediaError::InvalidOutput("no streams".into()));
    }

    // 표지 이미지(attached_pic)는 영상 스트림이 아니다 (MP3/M4A 앨범 아트).
    let v = raw
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0);
    let a = raw
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    let video = v.and_then(|s| {
        let rotation = s
            .side_data_list
            .iter()
            .find_map(|d| d.rotation)
            .or_else(|| s.tags.as_ref().and_then(|t| num::<f64>(&t.rotate)))
            .map(normalize_rotation)
            .unwrap_or(0);
        Some(VideoInfo {
            codec: s.codec_name.clone()?,
            width: s.width.filter(|w| *w > 0)?,
            height: s.height.filter(|h| *h > 0)?,
            rotation,
            fps: fps(&s.avg_frame_rate),
        })
    });
    let audio = a.and_then(|s| {
        Some(AudioInfo {
            codec: s.codec_name.clone()?,
            sample_rate: num(&s.sample_rate),
            channels: s.channels,
        })
    });
    // 컨테이너 길이가 없으면(MKV 일부 등) 스트림 길이로 보충한다.
    let duration_secs = secs(&format.duration)
        .or_else(|| v.and_then(|s| secs(&s.duration)))
        .or_else(|| a.and_then(|s| secs(&s.duration)));
    let creation_time = format
        .tags
        .as_ref()
        .and_then(|t| t.creation_time.clone())
        .or_else(|| {
            v.and_then(|s| s.tags.as_ref())
                .and_then(|t| t.creation_time.clone())
        });

    Ok(MediaProbe {
        container: format.format_name.unwrap_or_default(),
        duration_secs,
        size_bytes: num(&format.size),
        bit_rate: num(&format.bit_rate),
        creation_time,
        video,
        audio,
    })
}

impl MediaTools {
    /// ffprobe 로 메타데이터를 읽는다. 원본은 읽기 전용으로만 열린다.
    pub fn probe(&self, path: &Path) -> Result<MediaProbe> {
        let args: Vec<OsString> = vec![
            "-v".into(),
            "error".into(),
            "-print_format".into(),
            "json".into(),
            "-show_format".into(),
            "-show_streams".into(),
            "-i".into(),
            path.as_os_str().to_os_string(),
        ];
        let out = self.run(Tool::Ffprobe, &args, MAX_PROBE_OUTPUT)?;
        parse_probe_json(&out)
    }
}
