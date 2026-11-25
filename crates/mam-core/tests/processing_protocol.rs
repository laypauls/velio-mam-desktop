//! 커스텀 프로토콜 응답 테스트 (T-proc-08): `mamthumb`(썸네일·스트립)와 `mammedia`(원본 Range 스트리밍).
//!
//! 분류: Validation (잘못된 id·크기·경로 탈출 시도 -> 400/404) / Happy (200/206/304) / Boundary (Range 경계, 청크 상한) /
//! Business rule (오프라인·파일 없음·비활성 자산 -> 404, 원본 불변).

mod common;

use std::fs;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::protocol::{MAX_RANGE_CHUNK, serve_media, serve_thumb};
use mam_media::{ThumbSize, strip_path, thumbnail_path};

fn put_thumb(cache: &std::path::Path, id: i64, size: ThumbSize, bytes: &[u8]) {
    let p = thumbnail_path(cache, id, size);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

#[test]
fn t_proc_08_thumb_serves_existing_files_with_cache_headers_and_conditional_get() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path();
    put_thumb(cache, 7, ThumbSize::Small, b"RIFFsmall");
    put_thumb(cache, 7, ThumbSize::Large, b"RIFFlarge-bytes");
    let strip = strip_path(cache, 7);
    fs::create_dir_all(strip.parent().unwrap()).unwrap();
    fs::write(&strip, b"RIFFstrip").unwrap();

    // Act
    let small = serve_thumb(cache, "/7/320", None);
    let large = serve_thumb(cache, "/7/1024", None);
    let strip_resp = serve_thumb(cache, "/7/strip", None);
    let etag = small.header("ETag").unwrap().to_string();
    let not_modified = serve_thumb(cache, "/7/320", Some(&etag));
    let stale = serve_thumb(cache, "/7/320", Some("\"other\""));

    // Assert
    assert_eq!(
        (small.status, small.body.as_slice()),
        (200, &b"RIFFsmall"[..])
    );
    assert_eq!(large.body, b"RIFFlarge-bytes");
    assert_eq!(strip_resp.body, b"RIFFstrip");
    assert_eq!(small.header("Content-Type"), Some("image/webp"));
    assert_eq!(small.header("Content-Length"), Some("9"));
    assert!(small.header("Cache-Control").unwrap().contains("max-age"));
    assert_eq!(small.header("Access-Control-Allow-Origin"), Some("*"));
    assert_eq!(not_modified.status, 304);
    assert!(not_modified.body.is_empty());
    assert_eq!(stale.status, 200);
}

#[test]
fn t_proc_08b_thumb_rejects_malformed_requests_and_cannot_escape_the_cache() {
    // Arrange: 캐시 밖(부모 폴더)에 비밀 파일
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(dir.path().join("secret.webp"), b"SECRET").unwrap();
    put_thumb(&cache, 1, ThumbSize::Small, b"ok");

    // Act / Assert: 숫자가 아닌 id, 0, 음수, 너무 긴 id, 알 수 없는 크기, 구성요소 개수 오류, 탈출 시도 -> 400
    for bad in [
        "/abc/320",
        "/0/320",
        "/-1/320",
        "/99999999999999999999/320",
        "/1/999",
        "/1/",
        "/1",
        "/",
        "",
        "/1/320/extra",
        "/../secret/320",
        "/1/../../secret",
        "/..%2Fsecret/320",
        "/1%2F../320",
        "/1/320.webp",
        "//320",
    ] {
        let r = serve_thumb(&cache, bad, None);
        assert_eq!(r.status, 400, "{bad:?}");
        assert!(
            r.body.is_empty() && r.header("Content-Type").is_none(),
            "{bad:?}"
        );
    }
    // 형식은 맞지만 파일이 없다 -> 404
    assert_eq!(serve_thumb(&cache, "/2/320", None).status, 404);
    assert_eq!(serve_thumb(&cache, "/1/1024", None).status, 404);
    assert_eq!(serve_thumb(&cache, "/1/strip", None).status, 404);
    // 정상 요청은 영향 없음
    assert_eq!(serve_thumb(&cache, "/1/320", None).status, 200);
}

struct Lib {
    env: Env,
    id: i64,
    bytes: Vec<u8>,
}

/// 파일 하나를 스캔해 자산으로 등록한다.
fn lib_with(name: &str, bytes: Vec<u8>) -> Lib {
    let env = Env::new();
    let path = env.path(&format!("lib/{name}"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, &bytes).unwrap();
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    mam_core::scan::scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false))
        .unwrap();
    let id = env.count("SELECT asset_id FROM media_assets");
    Lib { env, id, bytes }
}

fn media(lib: &Lib, path: &str, range: Option<&str>) -> mam_core::protocol::ProtoResponse {
    serve_media(&lib.env.pool, path, range)
}

#[test]
fn t_proc_08c_media_serves_whole_file_and_byte_ranges() {
    // Arrange
    let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let lib = lib_with("clip.mp4", bytes.clone());
    let url = format!("/{}", lib.id);

    // Act
    let whole = media(&lib, &url, None);
    let head = media(&lib, &url, Some("bytes=0-9"));
    let mid = media(&lib, &url, Some("bytes=100-199"));
    let open = media(&lib, &url, Some("bytes=990-"));
    let tail = media(&lib, &url, Some("bytes=-5"));
    let clipped = media(&lib, &url, Some("bytes=995-5000"));
    let past = media(&lib, &url, Some("bytes=1000-"));
    let ignored = media(&lib, &url, Some("bytes=0-1,5-6"));

    // Assert
    assert_eq!((whole.status, whole.body.len()), (200, 1000));
    assert_eq!(whole.header("Content-Type"), Some("video/mp4"));
    assert_eq!(whole.header("Accept-Ranges"), Some("bytes"));
    assert_eq!(whole.header("Content-Range"), None);
    assert_eq!(head.status, 206);
    assert_eq!(head.body, bytes[..10]);
    assert_eq!(head.header("Content-Range"), Some("bytes 0-9/1000"));
    assert_eq!(head.header("Content-Length"), Some("10"));
    assert_eq!(mid.body, bytes[100..200]);
    assert_eq!(open.body, bytes[990..]);
    assert_eq!(open.header("Content-Range"), Some("bytes 990-999/1000"));
    assert_eq!(tail.body, bytes[995..]);
    assert_eq!(clipped.body, bytes[995..]);
    assert_eq!(clipped.header("Content-Range"), Some("bytes 995-999/1000"));
    assert_eq!(past.status, 416);
    assert_eq!(past.header("Content-Range"), Some("bytes */1000"));
    assert!(past.body.is_empty());
    // 다중 구간은 지원하지 않고 전체로 응답한다
    assert_eq!((ignored.status, ignored.body.len()), (200, 1000));
    assert!(
        whole
            .header("Access-Control-Expose-Headers")
            .unwrap()
            .contains("Content-Range")
    );
}

#[test]
fn t_proc_08d_media_caps_one_response_to_the_chunk_limit() {
    // Arrange: 청크 상한보다 큰 파일
    let size = MAX_RANGE_CHUNK as usize + 1_000_000;
    let lib = lib_with("long.wav", vec![7u8; size]);
    let url = format!("/{}", lib.id);

    // Act: 브라우저의 전형적인 요청 "bytes=0-"
    let first = media(&lib, &url, Some("bytes=0-"));
    let rest = media(&lib, &url, Some(&format!("bytes={MAX_RANGE_CHUNK}-")));

    // Assert: 첫 응답은 상한까지만, 다음 구간은 이어서
    assert_eq!(first.status, 206);
    assert_eq!(first.body.len() as u64, MAX_RANGE_CHUNK);
    assert_eq!(
        first.header("Content-Range").unwrap(),
        format!("bytes 0-{}/{size}", MAX_RANGE_CHUNK - 1)
    );
    assert_eq!(rest.status, 206);
    assert_eq!(rest.body.len(), 1_000_000);
    assert_eq!(first.header("Content-Type"), Some("audio/wav"));
}

#[test]
fn t_proc_08e_media_is_404_for_offline_missing_inactive_and_unknown_assets_and_400_for_bad_ids() {
    // Arrange
    let lib = lib_with("a.mp4", vec![1u8; 64]);
    let url = format!("/{}", lib.id);
    assert_eq!(media(&lib, &url, None).status, 200);

    // Assert: 형식 오류 -> 400 (경로 탈출 문자열 포함)
    for bad in [
        "/abc", "/0", "/-3", "/1/2", "/../x", "/1.mp4", "", "/%2e%2e",
    ] {
        assert_eq!(media(&lib, bad, None).status, 400, "{bad:?}");
    }
    // 없는 자산 -> 404
    assert_eq!(media(&lib, "/987654", None).status, 404);

    // 파일 없음(missing) -> 404
    lib.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE file_locations SET presence = 'missing'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(media(&lib, &url, None).status, 404);
    lib.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE file_locations SET presence = 'present'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(media(&lib, &url, None).status, 200);

    // 오프라인 볼륨 -> 404
    lib.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();
    assert_eq!(media(&lib, &url, None).status, 404);
    lib.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE volumes SET availability = 'online'", [])?;
            Ok(())
        })
        .unwrap();

    // 비활성 자산 -> 404
    for status in ["superseded", "deleted"] {
        lib.env
            .pool
            .write(mam_db::Priority::High, move |c| {
                c.execute("UPDATE media_assets SET status = ?1", [status])?;
                Ok(())
            })
            .unwrap();
        assert_eq!(media(&lib, &url, None).status, 404, "{status}");
    }
    lib.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE media_assets SET status = 'active'", [])?;
            Ok(())
        })
        .unwrap();

    // DB 는 present 인데 디스크에서 사라졌다 -> 404 (오류 본문·경로 노출 없음)
    fs::remove_file(lib.env.path("lib/a.mp4")).unwrap();
    let gone = media(&lib, &url, None);
    assert_eq!(gone.status, 404);
    assert!(gone.body.is_empty());
}

#[test]
fn t_proc_08f_media_is_read_only_and_leaves_the_original_untouched() {
    // Arrange: 읽기 전용 속성 파일 - 쓰기 핸들을 열면 실패하므로 읽기 전용으로만 열어야 서비스된다
    let lib = lib_with("ro.mp4", vec![9u8; 4096]);
    let path = lib.env.path("lib/ro.mp4");
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms).unwrap();
    let before = snapshot(&lib.env.vol);

    // Act
    let r1 = media(&lib, &format!("/{}", lib.id), None);
    let r2 = media(&lib, &format!("/{}", lib.id), Some("bytes=10-20"));

    // Assert
    assert_eq!((r1.status, r1.body.len()), (200, lib.bytes.len()));
    assert_eq!((r2.status, r2.body.len()), (206, 11));
    assert_eq!(snapshot(&lib.env.vol), before);
    let mut perms = fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
}

#[test]
fn t_proc_08g_preflight_allows_the_headers_the_webview_sends_without_touching_files() {
    // Act
    let r = mam_core::protocol::preflight();

    // Assert
    assert_eq!(r.status, 204);
    assert!(r.body.is_empty());
    assert_eq!(r.header("Access-Control-Allow-Origin"), Some("*"));
    let headers = r.header("Access-Control-Allow-Headers").unwrap();
    assert!(headers.contains("Range") && headers.contains("If-None-Match"));
    assert!(
        r.header("Access-Control-Allow-Methods")
            .unwrap()
            .contains("GET")
    );
}
