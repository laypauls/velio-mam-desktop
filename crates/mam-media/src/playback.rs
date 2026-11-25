//! WebView2(Chromium) 재생 가능 판정 (D7) — 컨테이너·코덱 조합 표.
//!
//! 표는 Chromium 공식 지원 범위에 기반한 **추정**이다. 실제 재생은 WebView2 런타임 버전·하드웨어·
//! 코덱 확장에 따라 달라지므로 `Conditional` 은 "시도해 볼 수 있으나 보장하지 않음"을 뜻한다.

use crate::probe::MediaProbe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// 재생 가능.
    Playable,
    /// 환경(하드웨어·확장·비공식 지원)에 따라 가능.
    Conditional,
    /// 불가 — "외부 플레이어로 열기" 또는 프록시가 필요하다.
    NotPlayable,
}

/// 판정 결과와 사유(로그·UI 번역 키 후보).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackVerdict {
    pub playback: Playback,
    pub reason: &'static str,
}

fn has(container: &str, name: &str) -> bool {
    container.split(',').any(|c| c == name)
}

fn video_support(codec: &str) -> Option<(Playback, &'static str)> {
    match codec {
        "h264" | "vp8" | "vp9" | "av1" => Some((Playback::Playable, "ok")),
        "hevc" => Some((
            Playback::Conditional,
            "hevc depends on hardware decoder/extension",
        )),
        _ => None,
    }
}

fn audio_support(codec: &str) -> bool {
    matches!(
        codec,
        "aac"
            | "mp3"
            | "opus"
            | "vorbis"
            | "flac"
            | "pcm_s16le"
            | "pcm_s24le"
            | "pcm_u8"
            | "pcm_f32le"
    )
}

/// 컨테이너(ffprobe `format_name`)와 코덱으로 재생 가능 여부를 판정한다.
pub fn webview_playback(
    container: &str,
    video_codec: Option<&str>,
    audio_codec: Option<&str>,
) -> PlaybackVerdict {
    let v = |playback, reason| PlaybackVerdict { playback, reason };
    let not = |reason| v(Playback::NotPlayable, reason);

    // 컨테이너 단계
    let mp4_family = has(container, "mp4") || has(container, "mov");
    let webm_family = has(container, "webm") || has(container, "matroska");
    let audio_container = has(container, "mp3")
        || has(container, "wav")
        || has(container, "flac")
        || has(container, "ogg")
        || has(container, "aac");
    if !(mp4_family || webm_family || audio_container) {
        return not("container not supported by WebView2");
    }

    // 코덱 단계
    let mut result = v(Playback::Playable, "ok");
    if let Some(vc) = video_codec {
        match video_support(vc) {
            Some((p, why)) => result = v(p, why),
            None => return not("video codec not supported by WebView2"),
        }
        // Ogg 안의 영상(Theora)은 지원하지 않는다.
        if has(container, "ogg") {
            return not("video in ogg container not supported");
        }
    }
    if let Some(ac) = audio_codec
        && !audio_support(ac)
    {
        return not("audio codec not supported by WebView2");
    }
    // ffprobe 는 MKV 와 WebM 을 모두 `matroska,webm` 으로 보고한다. WebM 허용 외 코덱(H.264 등)은
    // 공식 지원이 아니다 (Chromium 은 대개 재생하지만 보장하지 않음).
    if webm_family && video_codec.is_some_and(|c| !matches!(c, "vp8" | "vp9" | "av1")) {
        return v(
            Playback::Conditional,
            "non-webm codec in matroska/webm container",
        );
    }
    result
}

/// ffprobe 결과로 판정한다.
pub fn playback_of(probe: &MediaProbe) -> PlaybackVerdict {
    webview_playback(
        &probe.container,
        probe.video.as_ref().map(|v| v.codec.as_str()),
        probe.audio.as_ref().map(|a| a.codec.as_str()),
    )
}

/// `<img>` 로 원본을 직접 표시할 수 있는가 (확장자 기준, 추정).
/// JPEG/PNG/GIF/WebP/BMP/AVIF 는 Chromium 이 디코딩한다. TIFF/HEIC/RAW 는 변환한 썸네일·프리뷰로 표시해야 한다.
pub fn image_in_webview(extension: &str) -> Playback {
    match extension.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "avif" => Playback::Playable,
        _ => Playback::NotPlayable,
    }
}
