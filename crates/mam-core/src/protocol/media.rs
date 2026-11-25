//! `mammedia` - 원본 미디어 재생용 스트리밍 (HTTP Range). 원본은 읽기 전용으로만 연다.
//!
//! 응답 본문이 메모리 버퍼이므로 한 응답의 크기를 제한한다: Range 요청은 최대 [`MAX_RANGE_CHUNK`] 만 돌려주고(206 - 브라우저가
//! 이어서 다음 구간을 요청한다), Range 없는 요청은 [`WHOLE_FILE_LIMIT`] 이하만 통째로(200) 돌려준다. 더 큰 파일은 첫 구간을 206 으로 준다.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use mam_db::DbPool;

use super::{ProtoResponse, common_headers, parse_asset_id};
use crate::processing::{Source, load_source};

/// 한 응답에 담는 최대 구간 (8 MiB)
pub const MAX_RANGE_CHUNK: u64 = 8 * 1024 * 1024;
/// Range 없는 요청에 통째로 응답하는 최대 파일 크기 (64 MiB)
pub const WHOLE_FILE_LIMIT: u64 = 64 * 1024 * 1024;

/// 단일 Range 헤더의 해석 결과.
#[derive(Debug, PartialEq, Eq)]
enum RangeSpec {
    /// 없거나, 해석할 수 없거나, 여러 구간 - 전체 요청으로 취급
    Whole,
    /// `[start, end]` (닫힌 구간, 파일 길이 안으로 자른 값)
    Bytes(u64, u64),
    /// 만족할 수 없음 (시작이 끝을 넘음) -> 416
    Unsatisfiable,
}

fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeSpec::Whole;
    };
    if spec.contains(',') {
        return RangeSpec::Whole;
    }
    let Some((a, b)) = spec.split_once('-') else {
        return RangeSpec::Whole;
    };
    let (a, b) = (a.trim(), b.trim());
    let parse = |s: &str| s.parse::<u64>().ok();
    match (a.is_empty(), b.is_empty()) {
        // bytes=-N : 끝에서 N 바이트
        (true, false) => match parse(b) {
            Some(n) if n > 0 && len > 0 => RangeSpec::Bytes(len.saturating_sub(n), len - 1),
            Some(_) => RangeSpec::Unsatisfiable,
            None => RangeSpec::Whole,
        },
        // bytes=A- / bytes=A-B
        (false, _) => {
            let Some(start) = parse(a) else {
                return RangeSpec::Whole;
            };
            let end = if b.is_empty() {
                u64::MAX
            } else {
                match parse(b) {
                    Some(e) if e >= start => e,
                    _ => return RangeSpec::Whole, // RFC 9110: 잘못된 구간은 무시
                }
            };
            if start >= len {
                RangeSpec::Unsatisfiable
            } else {
                RangeSpec::Bytes(start, end.min(len - 1))
            }
        }
        (true, true) => RangeSpec::Whole,
    }
}

fn content_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mpg" | "mpeg" => "video/mpeg",
        "3gp" => "video/3gpp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "aif" | "aiff" | "aifc" => "audio/aiff",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "tif" | "tiff" => "image/tiff",
        "heic" | "heif" => "image/heic",
        _ => "application/octet-stream",
    }
}

fn read_range(file: &mut File, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(start))?;
    let want = end - start + 1;
    let mut buf = Vec::with_capacity(usize::try_from(want).unwrap_or(0));
    file.take(want).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `/<asset_id>` 의 원본을 응답한다. 잘못된 id 는 400, 자산이 없거나 활성이 아니거나 오프라인·파일 없음이면 404.
pub fn serve_media(pool: &DbPool, uri_path: &str, range: Option<&str>) -> ProtoResponse {
    let id_text = uri_path.trim_matches('/');
    let Some(asset_id) = parse_asset_id(id_text) else {
        return ProtoResponse::error(400);
    };
    let path = match pool.read(|c| Ok(load_source(c, asset_id))) {
        Ok(Ok(Source::Ready(_, path))) => path,
        Ok(Ok(_)) => return ProtoResponse::error(404),
        _ => return ProtoResponse::error(500),
    };
    let Ok(mut file) = File::open(&path) else {
        return ProtoResponse::error(404);
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return ProtoResponse::error(404);
    };
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut headers = common_headers();
    headers.push(("Content-Type", content_type(&ext).into()));
    headers.push(("Accept-Ranges", "bytes".into()));
    headers.push(("Cache-Control", "private, no-cache".into()));

    let (start, end, partial) = match parse_range(range, len) {
        RangeSpec::Unsatisfiable => {
            headers.push(("Content-Range", format!("bytes */{len}")));
            return ProtoResponse {
                status: 416,
                headers,
                body: Vec::new(),
            };
        }
        RangeSpec::Bytes(s, e) => (s, e.min(s + MAX_RANGE_CHUNK - 1), true),
        RangeSpec::Whole if len <= WHOLE_FILE_LIMIT => (0, len.saturating_sub(1), false),
        RangeSpec::Whole => (0, MAX_RANGE_CHUNK - 1, true),
    };
    let body = if len == 0 {
        Vec::new()
    } else {
        match read_range(&mut file, start, end) {
            Ok(b) => b,
            Err(_) => return ProtoResponse::error(404),
        }
    };
    if partial {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
    }
    headers.push(("Content-Length", body.len().to_string()));
    ProtoResponse {
        status: if partial { 206 } else { 200 },
        headers,
        body,
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary (Range 해석) / Validation
    use super::*;

    #[test]
    fn t_proc_range_01_parses_single_ranges() {
        assert_eq!(parse_range(Some("bytes=0-9"), 100), RangeSpec::Bytes(0, 9));
        assert_eq!(
            parse_range(Some("bytes=10-"), 100),
            RangeSpec::Bytes(10, 99)
        );
        assert_eq!(parse_range(Some("bytes=-5"), 100), RangeSpec::Bytes(95, 99));
        // 끝이 길이를 넘으면 자른다
        assert_eq!(
            parse_range(Some("bytes=90-500"), 100),
            RangeSpec::Bytes(90, 99)
        );
        // 접미 길이가 파일보다 길면 처음부터
        assert_eq!(
            parse_range(Some("bytes=-500"), 100),
            RangeSpec::Bytes(0, 99)
        );
    }

    #[test]
    fn t_proc_range_02_unsatisfiable_and_ignored_forms() {
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            RangeSpec::Unsatisfiable
        );
        assert_eq!(parse_range(Some("bytes=0-0"), 0), RangeSpec::Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-0"), 100), RangeSpec::Unsatisfiable);
        for ignored in [
            None,
            Some("items=0-1"),
            Some("bytes=5-2"),
            Some("bytes=a-b"),
            Some("bytes=0-1,5-6"),
            Some("bytes=-"),
            Some("bytes=0"),
        ] {
            assert_eq!(parse_range(ignored, 100), RangeSpec::Whole, "{ignored:?}");
        }
    }
}
