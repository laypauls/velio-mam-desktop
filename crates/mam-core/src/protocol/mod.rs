//! 커스텀 프로토콜 응답 로직 (P2-3 D5) — Tauri 와 무관한 순수 함수라 command 없이 시험할 수 있다.
//!
//! - `mamthumb`: 썸네일·프레임 스트립 파일을 그대로 응답한다 (base64 IPC 금지). 없으면 404
//! - `mammedia`: 원본 미디어를 **읽기 전용**으로 열어 Range(206)로 응답한다. 오프라인·파일 없음·비활성 자산은 404
//!
//! 요청 경로의 id 는 숫자만 허용하고, 파일 경로는 DB(자산 위치) 또는 `asset_id` 로 계산한 캐시 경로에서만 만든다 -
//! URL 의 어떤 문자열도 파일 경로가 되지 않으므로 경로 탈출이 구조적으로 불가능하다.

mod media;

use std::path::Path;
use std::time::UNIX_EPOCH;

use mam_media::{ThumbSize, strip_path, thumbnail_path};

pub use media::{MAX_RANGE_CHUNK, WHOLE_FILE_LIMIT, serve_media};

pub const THUMB_SCHEME: &str = "mamthumb";
pub const MEDIA_SCHEME: &str = "mammedia";

/// 프로토콜 응답 (Tauri 의 `http::Response` 로 옮겨 담는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoResponse {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl ProtoResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// 본문 없는 오류 응답. 내부 경로·사유는 싣지 않는다.
    pub(crate) fn error(status: u16) -> Self {
        Self {
            status,
            headers: common_headers(),
            body: Vec::new(),
        }
    }
}

/// 모든 응답에 붙는 헤더 - 개발 서버(`http://localhost:1420`)·`tauri.localhost` 어느 출처에서도 읽을 수 있게 한다.
pub(crate) fn common_headers() -> Vec<(&'static str, String)> {
    vec![
        ("Access-Control-Allow-Origin", "*".into()),
        (
            "Access-Control-Expose-Headers",
            "Content-Range, Accept-Ranges, Content-Length, ETag".into(),
        ),
        ("Cross-Origin-Resource-Policy", "cross-origin".into()),
        ("X-Content-Type-Options", "nosniff".into()),
    ]
}

/// `"123"` -> 123. 숫자만, 1~18자리, 0 불가 (i64 범위 안).
pub(crate) fn parse_asset_id(s: &str) -> Option<i64> {
    if s.is_empty() || s.len() > 18 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok().filter(|id| *id > 0)
}

/// `/<asset_id>/<size>` (size = `320` | `1024` | `strip`). 잘못된 id·크기는 400, 파일이 없으면 404.
/// 응답은 `Cache-Control: private, max-age=300` + `ETag`(크기-수정시각) 이고 `If-None-Match` 가 같으면 304.
pub fn serve_thumb(
    cache_root: &Path,
    uri_path: &str,
    if_none_match: Option<&str>,
) -> ProtoResponse {
    let mut parts = uri_path.trim_start_matches('/').split('/');
    let (Some(id), Some(size), None) = (parts.next(), parts.next(), parts.next()) else {
        return ProtoResponse::error(400);
    };
    let Some(asset_id) = parse_asset_id(id) else {
        return ProtoResponse::error(400);
    };
    let path = match size {
        "320" => thumbnail_path(cache_root, asset_id, ThumbSize::Small),
        "1024" => thumbnail_path(cache_root, asset_id, ThumbSize::Large),
        "strip" => strip_path(cache_root, asset_id),
        _ => return ProtoResponse::error(400),
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return ProtoResponse::error(404);
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis());
    let etag = format!("\"{:x}-{:x}\"", meta.len(), modified);
    let mut headers = common_headers();
    headers.push(("ETag", etag.clone()));
    headers.push(("Cache-Control", "private, max-age=300".into()));
    if if_none_match.is_some_and(|v| v.split(',').any(|t| t.trim() == etag)) {
        return ProtoResponse {
            status: 304,
            headers,
            body: Vec::new(),
        };
    }
    let Ok(body) = std::fs::read(&path) else {
        return ProtoResponse::error(404);
    };
    headers.push(("Content-Type", "image/webp".into()));
    headers.push(("Content-Length", body.len().to_string()));
    ProtoResponse {
        status: 200,
        headers,
        body,
    }
}

/// CORS 사전 요청(`OPTIONS`)에 대한 응답. 개발 서버 출처(`http://localhost:1420`)에서 `fetch` 로 `Range`/`If-None-Match`
/// 헤더를 보낼 때 필요하다 (`<img>`/`<video>` 는 사전 요청이 없다). 본문 없음, 파일 접근 없음.
pub fn preflight() -> ProtoResponse {
    let mut headers = common_headers();
    headers.push(("Access-Control-Allow-Methods", "GET, HEAD, OPTIONS".into()));
    headers.push((
        "Access-Control-Allow-Headers",
        "Range, If-None-Match, Content-Type".into(),
    ));
    headers.push(("Access-Control-Max-Age", "600".into()));
    ProtoResponse {
        status: 204,
        headers,
        body: Vec::new(),
    }
}
