//! 형식 판별 — 확장자 + 매직 바이트. 파일은 **읽기 전용**으로 앞부분만 읽는다.
//!
//! 확장자 지원은 디코딩 가능을 뜻하지 않는다. 여기서는 "어떤 종류의 미디어로 취급할지"만 정한다.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::{MediaError, Result};

/// 미디어 종류 (문서 파일은 대상 아님 — `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

/// 매직 바이트로 식별한 컨테이너 계열.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Jpeg,
    Png,
    Gif,
    Webp,
    Bmp,
    /// TIFF 구조 — TIFF 자체와 DNG/CR2/NEF/ARW 등 대부분의 RAW.
    Tiff,
    Heif,
    Avif,
    /// ISO BMFF (MP4/MOV/M4A 등). 영상/음성 구분은 ffprobe 몫.
    IsoBmff,
    /// Matroska/WebM.
    Matroska,
    Avi,
    Wav,
    /// ASF (WMV/WMA) — 영상/음성 모두 가능.
    Asf,
    MpegPs,
    MpegTs,
    Mxf,
    Mp3,
    Flac,
    Ogg,
    Aiff,
    AacAdts,
    /// 미디어가 아닌 것으로 알려진 형식 (PDF, ZIP/Office, OLE).
    Document,
}

impl Container {
    /// 이 계열이 속한 종류. 영상/음성 겸용(ISO BMFF·Matroska·ASF·Ogg)은 `None` — 확장자가 정한다.
    pub fn kind(self) -> Option<MediaKind> {
        use Container::*;
        match self {
            Jpeg | Png | Gif | Webp | Bmp | Tiff | Heif | Avif => Some(MediaKind::Image),
            Avi | MpegPs | MpegTs | Mxf => Some(MediaKind::Video),
            Wav | Mp3 | Flac | Aiff | AacAdts => Some(MediaKind::Audio),
            IsoBmff | Matroska | Asf | Ogg | Document => None,
        }
    }

    fn is_av_ambiguous(self) -> bool {
        matches!(
            self,
            Container::IsoBmff | Container::Matroska | Container::Asf | Container::Ogg
        )
    }
}

/// 확장자 -> 종류. 대소문자 무시, 점 없이.
pub fn kind_by_extension(ext: &str) -> Option<MediaKind> {
    let e = ext.to_ascii_lowercase();
    match e.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "heif"
        | "avif" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" | "pef"
        | "srw" => Some(MediaKind::Image),
        "mp4" | "m4v" | "mov" | "avi" | "mkv" | "wmv" | "webm" | "mpeg" | "mpg" | "mts"
        | "m2ts" | "mxf" | "flv" | "3gp" => Some(MediaKind::Video),
        "mp3" | "wav" | "aac" | "m4a" | "flac" | "ogg" | "oga" | "opus" | "wma" | "aiff"
        | "aif" | "aifc" => Some(MediaKind::Audio),
        _ => None,
    }
}

/// RAW 확장자인가 (내장 프리뷰 추출 대상).
pub fn is_raw_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" | "pef" | "srw"
    )
}

/// 판별 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    /// 최종 판정. `None` 이면 미디어 대상이 아니다.
    pub kind: Option<MediaKind>,
    /// 소문자 확장자 (없으면 빈 문자열).
    pub extension: String,
    pub by_extension: Option<MediaKind>,
    pub container: Option<Container>,
    /// 확장자와 내용이 다르다 (확장자 위장·잘못된 확장자).
    pub mismatch: bool,
}

const SNIFF_LEN: usize = 512;

/// 확장자와 매직 바이트로 판별한다. 내용이 우선하며, 내용을 모르면 확장자를 따른다.
pub fn detect(path: &Path) -> Result<Detected> {
    let mut head = Vec::with_capacity(SNIFF_LEN);
    File::open(path)
        .and_then(|f| f.take(SNIFF_LEN as u64).read_to_end(&mut head))
        .map_err(|e| MediaError::io(path, e))?;
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    Ok(classify(&extension, &head))
}

/// 확장자와 앞부분 바이트로 판정한다 (파일 I/O 없음).
pub fn classify(extension: &str, head: &[u8]) -> Detected {
    let by_extension = kind_by_extension(extension);
    let container = sniff(head);
    let (kind, mismatch) = match container {
        Some(Container::Document) => (None, by_extension.is_some()),
        Some(c) if c.is_av_ambiguous() => {
            // 영상/음성 겸용: 확장자가 영상·음성이면 따르고, 이미지 확장자 등은 위장으로 본다.
            match by_extension {
                Some(MediaKind::Video | MediaKind::Audio) => (by_extension, false),
                _ => (Some(MediaKind::Video), true),
            }
        }
        Some(c) => (c.kind(), c.kind() != by_extension),
        None => (by_extension, false),
    };
    Detected {
        kind,
        extension: extension.to_ascii_lowercase(),
        by_extension,
        container,
        mismatch,
    }
}

fn at(head: &[u8], off: usize, pat: &[u8]) -> bool {
    head.get(off..off + pat.len()) == Some(pat)
}

/// 매직 바이트 식별.
pub fn sniff(h: &[u8]) -> Option<Container> {
    use Container::*;
    if at(h, 0, &[0xFF, 0xD8, 0xFF]) {
        return Some(Jpeg);
    }
    if at(h, 0, &[0x89, b'P', b'N', b'G']) {
        return Some(Png);
    }
    if at(h, 0, b"GIF8") {
        return Some(Gif);
    }
    if at(h, 0, b"RIFF") {
        if at(h, 8, b"WEBP") {
            return Some(Webp);
        }
        if at(h, 8, b"AVI ") {
            return Some(Avi);
        }
        if at(h, 8, b"WAVE") {
            return Some(Wav);
        }
        return None;
    }
    if at(h, 0, b"BM") && h.len() >= 14 {
        return Some(Bmp);
    }
    if at(h, 0, b"II*\0") || at(h, 0, b"MM\0*") {
        return Some(Tiff);
    }
    if at(h, 4, b"ftyp") {
        let brand = h.get(8..12).unwrap_or(&[]);
        return Some(match brand {
            b"avif" | b"avis" => Avif,
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"heim" | b"heis" => Heif,
            _ => IsoBmff,
        });
    }
    if at(h, 0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(Matroska);
    }
    if at(h, 0, &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11]) {
        return Some(Asf);
    }
    if at(
        h,
        0,
        &[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01,
        ],
    ) {
        return Some(Mxf);
    }
    if at(h, 0, &[0x00, 0x00, 0x01, 0xBA]) {
        return Some(MpegPs);
    }
    // MPEG-TS: 188바이트 간격 0x47 동기 바이트 (M2TS 는 4바이트 접두 + 192바이트 간격).
    if h.len() > 188 * 2 && h[0] == 0x47 && h[188] == 0x47 && h[376] == 0x47 {
        return Some(MpegTs);
    }
    if h.len() > 4 + 192 * 2 && h[4] == 0x47 && h[196] == 0x47 && h[388] == 0x47 {
        return Some(MpegTs);
    }
    if at(h, 0, b"ID3")
        || (h.len() >= 2 && h[0] == 0xFF && h[1] & 0xE0 == 0xE0 && (h[1] >> 1) & 3 == 1)
    {
        return Some(Mp3);
    }
    if h.len() >= 2 && h[0] == 0xFF && (h[1] == 0xF1 || h[1] == 0xF9) {
        return Some(AacAdts);
    }
    if at(h, 0, b"fLaC") {
        return Some(Flac);
    }
    if at(h, 0, b"OggS") {
        return Some(Ogg);
    }
    if at(h, 0, b"FORM") && (at(h, 8, b"AIFF") || at(h, 8, b"AIFC")) {
        return Some(Aiff);
    }
    if at(h, 0, b"%PDF") || at(h, 0, b"PK\x03\x04") || at(h, 0, &[0xD0, 0xCF, 0x11, 0xE0]) {
        return Some(Document);
    }
    None
}
