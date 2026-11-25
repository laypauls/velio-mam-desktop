//! 커스텀 프로토콜 등록 (P2-3 D5): `mamthumb`(썸네일·프레임 스트립)와 `mammedia`(원본 Range 스트리밍).
//!
//! 응답 로직은 `mam_core::protocol` 의 순수 함수에 있고, 여기서는 Tauri 요청·응답 타입으로 옮기기만 한다.
//! 파일 I/O 는 `spawn_blocking` 에서 한다 - 웹뷰 스레드를 막지 않는다.
//!
//! Windows(WebView2) 의 URL 형태는 `http://<scheme>.localhost/<경로>` 이다 (macOS/Linux 는 `<scheme>://localhost/<경로>`).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use mam_core::protocol::{
    MEDIA_SCHEME, ProtoResponse, THUMB_SCHEME, preflight, serve_media, serve_thumb,
};
use mam_db::DbPool;
use tauri::http::{HeaderName, HeaderValue, Response, StatusCode};
use tauri::{Builder, Runtime};

/// 코어 응답 -> Tauri 응답. 잘못된 상태 코드·헤더 값은 건너뛰고 500 으로 대체한다 (패닉 금지).
pub fn to_http(r: ProtoResponse) -> Response<Cow<'static, [u8]>> {
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder().status(status);
    for (name, value) in r.headers {
        if let (Ok(n), Ok(v)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            builder = builder.header(n, v);
        }
    }
    builder.body(Cow::Owned(r.body)).unwrap_or_else(|_| {
        let mut fallback = Response::new(Cow::Borrowed(&[][..]));
        *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    })
}

fn header_text(req: &tauri::http::Request<Vec<u8>>, name: &str) -> Option<String> {
    req.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// 두 프로토콜을 빌더에 등록한다. `cache_root` = `MAMData/cache`.
pub fn register<R: Runtime>(
    builder: Builder<R>,
    cache_root: PathBuf,
    pool: Arc<DbPool>,
) -> Builder<R> {
    builder
        .register_asynchronous_uri_scheme_protocol(THUMB_SCHEME, move |_ctx, request, responder| {
            let cache = cache_root.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let is_options = request.method() == tauri::http::Method::OPTIONS;
                let resp = if is_options {
                    preflight()
                } else {
                    serve_thumb(
                        &cache,
                        request.uri().path(),
                        header_text(&request, "if-none-match").as_deref(),
                    )
                };
                responder.respond(to_http(resp));
            });
        })
        .register_asynchronous_uri_scheme_protocol(MEDIA_SCHEME, move |_ctx, request, responder| {
            let pool = Arc::clone(&pool);
            tauri::async_runtime::spawn_blocking(move || {
                let is_options = request.method() == tauri::http::Method::OPTIONS;
                let resp = if is_options {
                    preflight()
                } else {
                    serve_media(
                        &pool,
                        request.uri().path(),
                        header_text(&request, "range").as_deref(),
                    )
                };
                responder.respond(to_http(resp));
            });
        })
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Validation (T-ipc-19: 코어 응답 -> Tauri 응답 변환)
    use super::*;

    #[test]
    fn t_ipc_19_maps_status_headers_and_body() {
        // Arrange
        let core = ProtoResponse {
            status: 206,
            headers: vec![
                ("Content-Type", "video/mp4".to_string()),
                ("Content-Range", "bytes 0-3/10".to_string()),
            ],
            body: vec![1, 2, 3, 4],
        };

        // Act
        let r = to_http(core);

        // Assert
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.headers()["content-type"], "video/mp4");
        assert_eq!(r.headers()["content-range"], "bytes 0-3/10");
        assert_eq!(r.body().as_ref(), &[1, 2, 3, 4]);
    }

    #[test]
    fn t_ipc_19b_invalid_status_and_header_values_never_panic() {
        // Arrange: 범위 밖 상태 코드, 줄바꿈이 든 헤더 값(헤더 주입 시도)
        let core = ProtoResponse {
            status: 99,
            headers: vec![
                ("X-Evil", "a\r\nSet-Cookie: x=1".to_string()),
                ("ETag", "\"ok\"".to_string()),
            ],
            body: Vec::new(),
        };

        // Act
        let r = to_http(core);

        // Assert: 상태는 500 으로 대체, 잘못된 헤더는 버리고 나머지는 유지
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(r.headers().get("x-evil").is_none());
        assert_eq!(r.headers()["etag"], "\"ok\"");
    }
}
