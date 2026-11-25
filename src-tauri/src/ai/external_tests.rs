// Categories: Happy (분석 결과 변환) / Business rule (동의 전 호출 0회, 공급자·키 없음은 건너뜀, 경로 비전송) /
// External (401·429·5xx·오프라인 매핑, Retry-After) / Validation (iso 시각)
use std::path::PathBuf;
use std::sync::Arc;

use mam_ai::external::{HttpResponse, TransportError};
use mam_core::ai_jobs::{ExternalRequest, FrameCell, ImageInput, Transfer};

use super::super::test_support::{Fx, StubTransport, openai_reply, path_str};
use super::*;

fn request(fx: &Fx, asset: i64, media: &str, thumbs: Vec<PathBuf>) -> ExternalRequest {
    let _ = fx;
    ExternalRequest {
        asset_id: asset,
        media_type: media.into(),
        kinds: vec![StepKind::Describe, StepKind::Keywords],
        language: "ko".into(),
        transfer: Transfer::default(),
        images: thumbs
            .into_iter()
            .map(|path| ImageInput { path, frame: None })
            .collect(),
    }
}

fn adapter(fx: &Fx) -> ExternalAdapter {
    ExternalAdapter::new(
        Arc::clone(&fx.service.store),
        Arc::clone(&fx.service.vault),
        fx.transport.clone(),
        Arc::clone(&fx.pool),
        fx.dir.path().join("tmp"),
        Arc::clone(&fx.service.cli),
    )
}

#[test]
fn t_p3w_ext_01_no_request_before_consent() {
    // Arrange: 키는 있지만 동의가 없다
    let fx = Fx::new();
    fx.insert_asset(1, "image", "photos/a.jpg");
    fx.service
        .set_api_key("openai", "sk-test-1234", false)
        .unwrap();
    let thumb = fx.write_thumb("t1.jpg");
    // Act
    let err = adapter(&fx)
        .analyze(&request(&fx, 1, "image", vec![thumb]))
        .unwrap_err();
    // Assert
    assert_eq!(err.code, "consent_required");
    assert_eq!(fx.transport.count(), 0);
}

#[test]
fn t_p3w_ext_02_missing_provider_or_key_is_skipped_without_request() {
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service.give_consent().unwrap();
    let thumb = fx.write_thumb("t1.jpg");
    let a = adapter(&fx);
    // 공급자 없음
    let e = a
        .analyze(&request(&fx, 1, "image", vec![thumb.clone()]))
        .unwrap_err();
    assert_eq!(e.code, "disabled");
    // 공급자는 있으나 키 없음
    fx.service
        .update_settings(serde_json::json!({"defaultProvider": "gemini"}))
        .unwrap();
    let e = a
        .analyze(&request(&fx, 1, "image", vec![thumb]))
        .unwrap_err();
    assert_eq!(e.code, "disabled");
    assert_eq!(a.provider(), "gemini");
    assert_eq!(fx.transport.count(), 0);
}

#[test]
fn t_p3w_ext_03_success_maps_result_and_sends_no_absolute_path() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(7, "image", "private/dir/holiday.jpg");
    fx.consent_and_key();
    let thumb = fx.write_thumb("t7.jpg");
    // Act
    let out = adapter(&fx)
        .analyze(&request(&fx, 7, "image", vec![thumb.clone()]))
        .unwrap();
    // Assert
    assert_eq!(out.description, "a red square");
    assert_eq!(out.keywords, vec!["red", "square"]);
    assert_eq!(out.provider, "openai");
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), (12, 7));
    assert_eq!(out.usage.images, 1);
    assert!(out.usage.request_bytes > 0);
    assert_eq!(fx.transport.count(), 1, "재시도 루프 없음, 요청 1회");
    let (url, body) = fx.transport.seen.lock().unwrap()[0].clone();
    assert!(url.starts_with("https://api.openai.com/"));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("holiday.jpg"), "파일명은 전송");
    assert!(!text.contains("private"), "디렉터리는 전송하지 않는다");
    assert!(!text.contains(&path_str(thumb.parent().unwrap())));
    assert!(!text.contains("SECRET"));
}

#[test]
fn t_p3w_ext_04_provider_failures_map_to_core_codes() {
    type Reply = Box<dyn Fn() -> Result<HttpResponse, TransportError> + Send>;
    let cases: Vec<(Reply, &str)> = vec![
        (
            Box::new(|| {
                Ok(HttpResponse {
                    status: 401,
                    headers: vec![],
                    body: b"{}".to_vec(),
                })
            }),
            "auth_failed",
        ),
        (
            Box::new(|| {
                Ok(HttpResponse {
                    status: 400,
                    headers: vec![],
                    body: b"{}".to_vec(),
                })
            }),
            "bad_request",
        ),
        (
            Box::new(|| {
                Ok(HttpResponse {
                    status: 503,
                    headers: vec![],
                    body: b"{}".to_vec(),
                })
            }),
            "server_error",
        ),
        (
            Box::new(|| {
                Ok(HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: b"not json".to_vec(),
                })
            }),
            "bad_response",
        ),
        (Box::new(|| Err(TransportError::Offline)), "offline"),
        (Box::new(|| Err(TransportError::Timeout)), "timeout"),
    ];
    for (reply, code) in cases {
        let fx = Fx::new();
        *fx.transport.reply.lock().unwrap() = reply;
        fx.insert_asset(1, "image", "a.jpg");
        fx.consent_and_key();
        let thumb = fx.write_thumb("t.jpg");
        let e = adapter(&fx)
            .analyze(&request(&fx, 1, "image", vec![thumb]))
            .unwrap_err();
        assert_eq!(e.code, code);
        assert!(!e.message.contains("SECRET"), "{}", e.message);
        assert_eq!(fx.transport.count(), 1, "{code}: 어댑터 안 재시도 없음");
    }
}

#[test]
fn t_p3w_ext_05_rate_limit_carries_retry_after() {
    let fx = Fx::new();
    *fx.transport.reply.lock().unwrap() = Box::new(|| {
        Ok(HttpResponse {
            status: 429,
            headers: vec![("Retry-After".into(), "17".into())],
            body: b"{}".to_vec(),
        })
    });
    fx.insert_asset(1, "image", "a.jpg");
    fx.consent_and_key();
    let thumb = fx.write_thumb("t.jpg");
    let e = adapter(&fx)
        .analyze(&request(&fx, 1, "image", vec![thumb]))
        .unwrap_err();
    assert_eq!(e.code, "rate_limited");
    assert_eq!(e.retry_after_secs, Some(17));
    assert_eq!(fx.transport.count(), 1);
}

#[test]
fn t_p3w_ext_06_unreadable_thumbnail_is_retryable_code_and_no_request() {
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.consent_and_key();
    let e = adapter(&fx)
        .analyze(&request(
            &fx,
            1,
            "image",
            vec![fx.dir.path().join("missing.jpg")],
        ))
        .unwrap_err();
    assert_eq!(e.code, "thumbnail_missing");
    assert_eq!(fx.transport.count(), 0);
}

#[test]
fn t_p3w_ext_07_video_images_sent_counts_frame_limit() {
    let scope = TransferScope {
        image_max_edge: 1024,
        video_frames: 3,
        include_metadata: true,
    };
    assert_eq!(images_sent("video", 5, &scope), 3);
    assert_eq!(images_sent("video", 2, &scope), 2);
    assert_eq!(images_sent("image", 4, &scope), 1);
    assert_eq!(images_sent("image", 0, &scope), 0);
}

#[test]
fn t_p3w_ext_08_iso_utc_formats_epoch_and_leap_day() {
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_utc(951_782_400_000 + 3_723_000), "2000-02-29T01:02:03Z");
    assert_eq!(iso_utc(-1000), "1969-12-31T23:59:59Z");
}

#[test]
fn t_p3w_ext_09_reply_fixture_is_valid_openai_shape() {
    // 기록 응답 픽스처가 파서가 받는 형태인지 (다른 시험의 전제)
    let v: serde_json::Value = serde_json::from_str(&openai_reply()).unwrap();
    assert!(v["choices"][0]["message"]["content"].is_string());
    let _ = StubTransport::new(200, "{}");
}

#[test]
fn t_p3w_ext_10_video_frame_cells_are_cropped_sent_and_temp_removed() {
    // Arrange: 3칸 스트립 (칸마다 다른 색)
    let fx = Fx::new();
    fx.insert_asset(5, "video", "clips/a.mp4");
    fx.consent_and_key();
    let strip = fx.dir.path().join("cache").join("strip.png");
    std::fs::create_dir_all(strip.parent().unwrap()).unwrap();
    let mut img = image::RgbImage::new(3 * 16, 12);
    for (x, _, px) in img.enumerate_pixels_mut() {
        *px = image::Rgb([(x / 16 * 90) as u8, 20, 20]);
    }
    img.save(&strip).unwrap();
    let mut req = request(&fx, 5, "video", vec![]);
    req.images = (0..3)
        .map(|index| ImageInput {
            path: strip.clone(),
            frame: Some(FrameCell { index, cells: 3 }),
        })
        .collect();
    // Act
    let out = adapter(&fx).analyze(&req).unwrap();
    // Assert
    assert_eq!(out.usage.images, 3);
    assert_eq!(fx.transport.count(), 1);
    let left = std::fs::read_dir(fx.dir.path().join("tmp"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "잘라낸 임시 파일은 전송 뒤 지워진다");
    // 잘못된 칸 번호는 요청 없이 invalid_input
    req.images[0].frame = Some(FrameCell { index: 9, cells: 3 });
    let e = adapter(&fx).analyze(&req).unwrap_err();
    assert_eq!(e.code, "invalid_input");
    assert_eq!(fx.transport.count(), 1);
}

#[test]
fn t_capfb_ai_01_external_payload_uses_only_the_raw_capture_date_never_the_modified_fallback() {
    // Arrange: 설정 `capturedAtFallback` = modified(기본) 이고, 수정일(2022-01-15)이 있는 자산 2건 — 1건만 실제 촬영일(2024-03-10)이 있다
    let fx = Fx::new();
    fx.insert_asset(1, "image", "photos/with.jpg");
    fx.insert_asset(2, "image", "photos/without.jpg");
    fx.pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE media_assets SET modified_at = 1642248000000", [])?;
            c.execute(
                "INSERT INTO media_metadata (asset_id, captured_at) VALUES (1, 1710072000000)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let setting = fx
        .pool
        .read(|c| mam_db::settings::load(c).map(|(s, _)| s.captured_at_fallback))
        .unwrap();
    assert_eq!(
        setting,
        mam_db::CapturedAtFallback::Modified,
        "게이트 ON 상태에서 확인"
    );
    let a = adapter(&fx);

    // Act
    let with = a.meta(1, "image", true).unwrap();
    let without = a.meta(2, "image", true).unwrap();

    // Assert: 촬영일이 있으면 원본 값, 없으면 비어 있다 (수정일로 채우지 않는다)
    assert_eq!(with.captured_at.as_deref(), Some("2024-03-10T12:00:00Z"));
    assert_eq!(without.captured_at, None);
}
