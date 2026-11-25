//! 외부 전송 페이로드 (PL-A3) — 분류: Business rule / Boundary. 절대 경로 비노출, 이미지 긴 변·프레임 수 상한. 실호출 없음.

#[path = "external_support.rs"]
mod support;

use base64::Engine;
use mam_ai::external::{AiClient, AnalysisKind, ConsentGate, MediaType, Provider};
use serde_json::Value;
use support::*;

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

/// 본문에서 OpenAI data URL 이미지들을 디코드해 (가로, 세로) 목록으로 돌려준다.
fn sent_image_dims(body: &[u8]) -> Vec<(u32, u32)> {
    let v: Value = serde_json::from_slice(body).unwrap();
    v["messages"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["image_url"]["url"].as_str())
        .map(|u| {
            let b = base64::engine::general_purpose::STANDARD
                .decode(u.split("base64,").nth(1).unwrap())
                .unwrap();
            let img = image::load_from_memory(&b).unwrap();
            (img.width(), img.height())
        })
        .collect()
}

#[test]
fn t_ext_05_body_has_no_path_and_image_edge_is_bounded() {
    // Arrange
    let r = rig(
        Provider::OpenAi,
        vec![resp(200, &[], &openai_ok(MODEL_JSON))],
        true,
    );
    let mut req = request(vec![png(3000, 2000)]);
    req.scope.image_max_edge = 1024;
    // Act
    r.client.analyze(&gate(), &req).unwrap();
    // Assert
    let sent = r.transport.requests.lock().unwrap()[0].clone();
    let body = String::from_utf8(sent.body.clone()).unwrap();
    assert!(body.contains("beach.jpg"));
    for leak in ["Users", "alice", "C:\\\\", "C:/", "trip"] {
        assert!(!body.contains(leak), "leaked {leak}");
    }
    let dims = sent_image_dims(&sent.body);
    assert_eq!(dims.len(), 1);
    assert!(dims[0].0.max(dims[0].1) <= 1024 && dims[0].0 == 1024);
}

#[test]
fn t_ext_05_scope_is_clamped_and_frames_limited_and_metadata_optional() {
    // Arrange: 영상 프레임 8장, 범위 밖 설정값
    let frames: Vec<Vec<u8>> = (0..8).map(|i| png(600 + i, 400)).collect();
    let mut req = request(frames);
    req.meta.media_type = MediaType::Video;
    req.scope.video_frames = 3;
    req.scope.image_max_edge = 10; // -> 256
    req.scope.include_metadata = false;
    let r = rig(
        Provider::OpenAi,
        vec![resp(200, &[], &openai_ok(MODEL_JSON))],
        true,
    );
    // Act
    r.client.analyze(&gate(), &req).unwrap();
    // Assert
    let sent = r.transport.requests.lock().unwrap()[0].clone();
    let dims = sent_image_dims(&sent.body);
    assert_eq!(dims.len(), 3);
    assert!(dims.iter().all(|d| d.0.max(d.1) <= 256));
    assert!(
        !String::from_utf8(sent.body.clone())
            .unwrap()
            .contains("beach.jpg")
    );
    // 상한 초과도 보정: 프레임 99 -> 10
    let mut big = request((0..12).map(|_| png(300, 300)).collect());
    big.meta.media_type = MediaType::Video;
    big.scope.video_frames = 99;
    let r = rig(
        Provider::OpenAi,
        vec![resp(200, &[], &openai_ok(MODEL_JSON))],
        true,
    );
    r.client.analyze(&gate(), &big).unwrap();
    assert_eq!(
        sent_image_dims(&r.transport.requests.lock().unwrap()[0].body).len(),
        10
    );
}

#[test]
fn t_ext_05_audio_sends_text_only() {
    let r = rig(
        Provider::Claude,
        vec![resp(200, &[], &claude_ok(MODEL_JSON))],
        true,
    );
    let mut req = request(vec![]);
    req.meta.media_type = MediaType::Audio;
    req.kinds = vec![AnalysisKind::Summary];
    r.client.analyze(&gate(), &req).unwrap();
    let body = String::from_utf8(r.transport.requests.lock().unwrap()[0].body.clone()).unwrap();
    assert!(!body.contains("\"image\""));
}
