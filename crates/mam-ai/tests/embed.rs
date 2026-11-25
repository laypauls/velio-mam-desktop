//! T-ai-05~08, 14 - 임베딩. 분류: Happy / Business rule / Validation / Boundary.
//! 실제 모델이 필요한 시험은 `MAM_AI_TEST_MODELS`(제품 배치 `models/<id>/model.onnx`)가 없으면 사유를 출력하고 건너뛴다.

mod common;

use std::path::Path;

use common::require_models;
use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage};
use mam_ai::AiError;
use mam_ai::local::embed::image_pre::{
    CLIP_MEAN, CLIP_SIZE, CLIP_STD, CLIP_TENSOR_LEN, decode_bytes, decode_image, preprocess_clip,
};
use mam_ai::local::embed::{
    CLIP_IMAGE_DIM, CLIP_TEXT_DIM, ClipTextEmbedder, E5_DIM, E5Embedder, EmbedConfig,
    ImageEmbedder, TextRole,
};

const PLANE: usize = (CLIP_SIZE * CLIP_SIZE) as usize;

fn norm(c: usize, v: u8) -> f32 {
    (v as f32 / 255.0 - CLIP_MEAN[c]) / CLIP_STD[c]
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn l2(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn shape_image(kind: &str, color: [u8; 3]) -> DynamicImage {
    let mut img = RgbImage::from_pixel(320, 240, Rgb([235, 235, 235]));
    for y in 0..240i32 {
        for x in 0..320i32 {
            let (dx, dy) = (x - 160, y - 120);
            let inside = match kind {
                "circle" => dx * dx + dy * dy <= 80 * 80,
                "square" => dx.abs() <= 80 && dy.abs() <= 80,
                _ => {
                    let t = (dy + 80) as f32 / 160.0;
                    (0.0..=1.0).contains(&t) && (dx.abs() as f32) <= 80.0 * t
                }
            };
            if inside {
                img.put_pixel(x as u32, y as u32, Rgb(color));
            }
        }
    }
    DynamicImage::ImageRgb8(img)
}

// ---------- T-ai-05: 전처리 (모델 불필요) ----------

#[test]
fn t_ai_05_solid_color_matches_reference_normalization() {
    // Arrange: 300x200 단색 -> 리사이즈/크롭 후에도 모든 픽셀이 같은 값
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(300, 200, Rgb([200, 100, 50])));
    // Act
    let t = preprocess_clip(&img);
    // Assert
    assert_eq!(t.len(), CLIP_TENSOR_LEN);
    for (c, v) in [(0, 200u8), (1, 100), (2, 50)] {
        let want = norm(c, v);
        for &x in &t[c * PLANE..(c + 1) * PLANE] {
            assert!((x - want).abs() < 1e-4, "channel {c}: {x} vs {want}");
        }
    }
}

#[test]
fn t_ai_05_center_crop_keeps_middle_of_landscape() {
    // Arrange: 448x224 - 왼쪽 절반 빨강, 오른쪽 절반 파랑. 짧은 변이 이미 224 라 리사이즈 없이 가운데 224 열을 자른다.
    let mut img = RgbImage::new(448, 224);
    for (x, _, p) in img.enumerate_pixels_mut() {
        *p = if x < 224 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        };
    }
    // Act
    let t = preprocess_clip(&DynamicImage::ImageRgb8(img));
    // Assert: 자른 영역은 원본 x=112..336 -> 출력 x<112 빨강, x>=112 파랑
    let red_r = norm(0, 255);
    let blue_r = norm(0, 0);
    assert!((t[0] - red_r).abs() < 1e-4, "left edge should be red");
    assert!((t[111] - red_r).abs() < 1e-4);
    assert!((t[112] - blue_r).abs() < 1e-4, "boundary at x=112");
    assert!((t[223] - blue_r).abs() < 1e-4);
    // 파랑 채널은 반대
    assert!((t[2 * PLANE] - norm(2, 0)).abs() < 1e-4);
    assert!((t[2 * PLANE + 200] - norm(2, 255)).abs() < 1e-4);
}

#[test]
fn t_ai_05_downscale_portrait_and_alpha_are_handled() {
    // 세로로 긴 큰 이미지: 짧은 변 기준 리사이즈 후 중앙 자르기. 윗/아랫부분 색이 다른 이미지에서 중앙 색만 남는다.
    let mut img = RgbImage::new(448, 896);
    for (_, y, p) in img.enumerate_pixels_mut() {
        *p = if (224..672).contains(&y) {
            Rgb([0, 255, 0])
        } else {
            Rgb([255, 0, 0])
        };
    }
    let t = preprocess_clip(&DynamicImage::ImageRgb8(img));
    // 중앙 절반(= 리사이즈 후 y 112..336 중 자른 224 행)은 초록
    let mid = (112 * CLIP_SIZE + 112) as usize;
    assert!((t[PLANE + mid] - norm(1, 255)).abs() < 0.05);
    assert!((t[mid] - norm(0, 0)).abs() < 0.05);

    // 알파 채널은 버려진다 (불투명 RGB 로 취급) - 패닉 없이 같은 길이
    let rgba = DynamicImage::ImageRgba8(RgbaImage::from_pixel(10, 10, Rgba([10, 20, 30, 0])));
    let t = preprocess_clip(&rgba);
    assert_eq!(t.len(), CLIP_TENSOR_LEN);
    assert!((t[0] - norm(0, 10)).abs() < 1e-4);
}

#[test]
fn t_ai_05_tiny_one_pixel_image_is_upscaled() {
    let t = preprocess_clip(&DynamicImage::ImageRgb8(RgbImage::from_pixel(
        1,
        1,
        Rgb([9, 9, 9]),
    )));
    assert_eq!(t.len(), CLIP_TENSOR_LEN);
    assert!(t.iter().all(|x| x.is_finite()));
}

// ---------- T-ai-08: 손상 이미지 (모델 불필요) ----------

#[test]
fn t_ai_08_corrupt_and_missing_images_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    // 쓰레기 바이트
    let junk = dir.path().join("junk.jpg");
    std::fs::write(&junk, b"this is not an image at all").unwrap();
    assert!(matches!(decode_image(&junk), Err(AiError::Decode(_))));
    // 0바이트
    let empty = dir.path().join("empty.png");
    std::fs::write(&empty, b"").unwrap();
    assert!(matches!(decode_image(&empty), Err(AiError::Decode(_))));
    // 잘린 PNG
    let mut ok = Vec::new();
    DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, Rgb([1, 2, 3])))
        .write_to(&mut std::io::Cursor::new(&mut ok), image::ImageFormat::Png)
        .unwrap();
    let cut = dir.path().join("cut.png");
    std::fs::write(&cut, &ok[..ok.len() / 2]).unwrap();
    assert!(matches!(decode_image(&cut), Err(AiError::Decode(_))));
    assert!(matches!(decode_bytes(&ok[..20]), Err(AiError::Decode(_))));
    // 없는 파일은 I/O 오류
    assert!(matches!(
        decode_image(&dir.path().join("missing.jpg")),
        Err(AiError::Io { .. })
    ));
    // 정상 파일은 읽히고 원본은 바뀌지 않는다
    let good = dir.path().join("good.png");
    std::fs::write(&good, &ok).unwrap();
    let before = std::fs::read(&good).unwrap();
    assert!(decode_image(&good).is_ok());
    assert_eq!(std::fs::read(&good).unwrap(), before);
}

#[test]
fn t_ai_08_oversized_dimensions_are_rejected_before_decoding() {
    // 20,001 x 1 PNG (압축되어 작다) - 한 변 20,000px 상한 초과
    let mut bytes = Vec::new();
    DynamicImage::ImageRgb8(RgbImage::from_pixel(20_001, 1, Rgb([0, 0, 0])))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    assert!(matches!(decode_bytes(&bytes), Err(AiError::Decode(_))));
}

// ---------- 모델 필요 시험 ----------

fn cfg() -> EmbedConfig {
    EmbedConfig {
        threads: 4,
        batch_size: 4,
    }
}

fn vision(root: &Path) -> ImageEmbedder {
    ImageEmbedder::load(&root.join("clip-vit-b32-vision/model.onnx"), cfg()).expect("vision model")
}

fn clip_text(root: &Path) -> ClipTextEmbedder {
    let d = root.join("clip-b32-multilingual-text");
    ClipTextEmbedder::load(
        &d.join("model.onnx"),
        &d.join("tokenizer.json"),
        &d.join("dense.safetensors"),
        cfg(),
    )
    .expect("clip text model")
}

fn e5(root: &Path) -> E5Embedder {
    let d = root.join("multilingual-e5-small");
    E5Embedder::load(&d.join("model.onnx"), &d.join("tokenizer.json"), cfg()).expect("e5 model")
}

#[test]
fn t_ai_06_same_image_same_unit_vector() {
    let Some(root) = require_models("t_ai_06") else {
        return;
    };
    let emb = vision(&root);
    let img = shape_image("circle", [220, 30, 30]);

    let a = emb.embed_images(&[&img]).unwrap().remove(0);
    let b = emb.embed_images(&[&img]).unwrap().remove(0);

    assert_eq!(a.len(), CLIP_IMAGE_DIM);
    assert!((l2(&a) - 1.0).abs() < 1e-4, "L2 norm {}", l2(&a));
    assert!(
        dot(&a, &b) > 0.99999,
        "same input must give the same vector"
    );
    // 배치 구성과 무관해야 한다 (배치 불변성)
    let other = shape_image("square", [30, 60, 220]);
    let batch = emb.embed_images(&[&other, &img]).unwrap();
    assert!(
        dot(&batch[1], &a) > 0.9999,
        "batch invariance: {}",
        dot(&batch[1], &a)
    );
    assert_ne!(batch[0], batch[1]);
}

#[test]
fn t_ai_06_embed_file_reads_original_without_modifying_it() {
    let Some(root) = require_models("t_ai_06_file") else {
        return;
    };
    let emb = vision(&root);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.png");
    shape_image("triangle", [30, 160, 50]).save(&path).unwrap();
    let before = (
        std::fs::read(&path).unwrap(),
        std::fs::metadata(&path).unwrap().modified().unwrap(),
    );

    let v = emb.embed_file(&path).unwrap();

    assert_eq!(v.len(), 512);
    assert_eq!(std::fs::read(&path).unwrap(), before.0);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before.1
    );
    assert!(matches!(
        emb.embed_images(&[]),
        Err(AiError::InvalidInput(_))
    ));
}

#[test]
fn t_ai_07_korean_query_finds_matching_image() {
    let Some(root) = require_models("t_ai_07") else {
        return;
    };
    let (vis, txt) = (vision(&root), clip_text(&root));
    // 색과 도형이 모두 다른 4장
    let set = [
        ("빨간 원", shape_image("circle", [220, 30, 30])),
        ("파란 사각형", shape_image("square", [30, 60, 220])),
        ("초록 삼각형", shape_image("triangle", [30, 160, 50])),
        ("노란 원", shape_image("circle", [240, 220, 40])),
    ];
    let refs: Vec<&DynamicImage> = set.iter().map(|(_, i)| i).collect();
    let iv = vis.embed_images(&refs).unwrap();

    // "빨간 원"/"파란 사각형" 질의가 각각 자기 이미지에 가장 가깝다
    for (want, (query, _)) in set.iter().enumerate().take(2) {
        let q = txt.embed_text(query).unwrap();
        assert_eq!(q.len(), CLIP_TEXT_DIM);
        assert!((l2(&q) - 1.0).abs() < 1e-4);
        let best = (0..iv.len())
            .max_by(|&a, &b| dot(&q, &iv[a]).partial_cmp(&dot(&q, &iv[b])).unwrap())
            .unwrap();
        assert_eq!(best, want, "query {query:?} best={best}");
    }
    // 한국어와 영어 질의가 같은 의미면 서로 가깝다 (공동 공간)
    let ko = txt.embed_text("빨간 원").unwrap();
    let en = txt.embed_text("a red circle").unwrap();
    let unrelated = txt.embed_text("a blue square").unwrap();
    assert!(dot(&ko, &en) > dot(&ko, &unrelated));
}

#[test]
fn t_ai_07_e5_query_prefix_and_cross_lingual_retrieval() {
    let Some(root) = require_models("t_ai_07_e5") else {
        return;
    };
    let e5 = e5(&root);
    let passages = [
        "회의에서 다음 분기 제품 출시 일정과 예산을 논의했다",
        "A dog is running on the beach at sunset",
        "The chef prepared a traditional Korean dinner with kimchi stew",
    ];
    let pv = e5.embed(TextRole::Passage, &passages).unwrap();
    assert_eq!(pv[0].len(), E5_DIM);
    for v in &pv {
        assert!((l2(v) - 1.0).abs() < 1e-4);
    }
    for (query, want) in [
        ("dog playing at the seaside", 1),
        ("김치찌개 요리", 2),
        ("분기 제품 출시 회의", 0),
    ] {
        let q = e5.embed_query(query).unwrap();
        let best = (0..pv.len())
            .max_by(|&a, &b| dot(&q, &pv[a]).partial_cmp(&dot(&q, &pv[b])).unwrap())
            .unwrap();
        assert_eq!(best, want, "query {query:?}");
    }
    // 접두어 규칙: 같은 텍스트라도 query/passage 벡터는 다르다
    let q = e5.embed_query("hello world").unwrap();
    let p = e5.embed_passage("hello world").unwrap();
    assert!(dot(&q, &p) < 0.9999);
}

#[test]
fn t_ai_08_empty_and_oversized_text() {
    let Some(root) = require_models("t_ai_08") else {
        return;
    };
    let (txt, e5) = (clip_text(&root), e5(&root));

    // 빈 문자열/공백은 오류
    assert!(matches!(
        txt.embed_texts(&[""]),
        Err(AiError::InvalidInput(_))
    ));
    assert!(matches!(
        txt.embed_texts(&["ok", "   \n"]),
        Err(AiError::InvalidInput(_))
    ));
    assert!(matches!(
        txt.embed_texts(&[]),
        Err(AiError::InvalidInput(_))
    ));
    assert!(matches!(
        e5.embed(TextRole::Query, &[" "]),
        Err(AiError::InvalidInput(_))
    ));

    // 초장문은 토큰 상한에서 잘린다: 앞 200 단어가 같으면 뒤에 더 붙여도 같은 벡터 (CLIP 텍스트 상한 128 토큰)
    let head: String = (0..200).map(|i| format!("word{i} ")).collect();
    let long: String = head.clone() + &(0..2000).map(|i| format!("tail{i} ")).collect::<String>();
    let a = txt.embed_text(&head).unwrap();
    let b = txt.embed_text(&long).unwrap();
    assert!(
        dot(&a, &b) > 0.9999,
        "truncation should make them identical"
    );
    assert_eq!(txt.max_tokens(), 128);
    assert!((l2(&b) - 1.0).abs() < 1e-4);

    // e5 도 같은 방식 (512 토큰)
    let head: String = (0..700).map(|i| format!("word{i} ")).collect();
    let long = head.clone() + &"tail ".repeat(3000);
    let a = e5.embed_passage(&head).unwrap();
    let b = e5.embed_passage(&long).unwrap();
    assert!(dot(&a, &b) > 0.9999);

    // 한글·이모지·특수문자 혼합도 유한한 단위 벡터
    let v = txt
        .embed_text("🔥 한글 mixed ✓ <script>alert(1)</script>")
        .unwrap();
    assert!(v.iter().all(|x| x.is_finite()) && (l2(&v) - 1.0).abs() < 1e-4);
}

#[test]
fn t_ai_14_missing_model_file_is_reported_not_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let r = ImageEmbedder::load(&dir.path().join("nope/model.onnx"), EmbedConfig::default());
    assert!(matches!(r, Err(AiError::ModelMissing { .. })));
    // 깨진 모델 파일은 ONNX 오류
    let bad = dir.path().join("bad.onnx");
    std::fs::write(&bad, b"not an onnx model").unwrap();
    assert!(matches!(
        ImageEmbedder::load(&bad, EmbedConfig::default()),
        Err(AiError::Onnx(_))
    ));
}
