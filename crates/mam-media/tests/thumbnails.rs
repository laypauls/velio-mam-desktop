//! T-media-05 ~ 08, 11: 이미지 메타·방향, 경계(대형·1x1·투명), RAW 프리뷰, 원본 보호, 경로 샤딩.
//!
//! 분류: Happy / Boundary / Business rule / Validation. FFmpeg 없이(`no_tools`) 동작해야 하는 경로만 다룬다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use image::{ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use mam_media::error::MediaError;
use mam_media::raw_preview::extract_raw_preview;
use mam_media::read_image_meta;
use mam_media::thumbnail::{
    DecodeMethod, SMALL_EDGE, ThumbSize, ThumbnailOptions, create_thumbnails, thumbnail_path,
};

fn opts() -> ThumbnailOptions {
    ThumbnailOptions::default()
}

fn make(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

fn load(p: &Path) -> image::RgbaImage {
    image::open(p).unwrap().to_rgba8()
}

fn is_red(p: &image::Rgba<u8>) -> bool {
    p[0] > 200 && p[2] < 60
}
fn is_blue(p: &image::Rgba<u8>) -> bool {
    p[2] > 200 && p[0] < 60
}

// ---------------------------------------------------------------- T-media-05

#[test]
fn t_media_05_exif_meta_and_orientation_applied() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // Arrange: 64x32 (왼쪽 빨강/오른쪽 파랑) + EXIF 방향 6 (시계 방향 90도 회전해 보여야 함)
    let jpg = make(
        src_dir.path(),
        "rot.jpg",
        &jpeg_with_exif(&red_blue(64, 32), 6, "2024:05:01 12:34:56"),
    );

    // Act
    let meta = read_image_meta(&jpg).unwrap();
    let out = create_thumbnails(&no_tools(), &jpg, 1, cache.path(), &opts()).unwrap();

    // Assert: 메타
    assert_eq!((meta.width, meta.height), (Some(64), Some(32)));
    assert_eq!(meta.orientation, 6);
    assert_eq!(meta.taken_at.as_deref(), Some("2024-05-01T12:34:56"));
    // Assert: 썸네일은 32x64 로 회전, 위쪽이 빨강(원래 왼쪽) 아래쪽이 파랑
    assert_eq!(out.method, DecodeMethod::ImageCrate);
    assert_eq!(out.large_size, (32, 64));
    let img = load(&out.large);
    assert_eq!(img.dimensions(), (32, 64));
    assert!(is_red(img.get_pixel(16, 6)), "top should be red");
    assert!(is_blue(img.get_pixel(16, 58)), "bottom should be blue");
}

#[test]
fn t_media_05_orientation_variants_and_no_exif() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // 방향 1 (정상): 가로 유지, 왼쪽 빨강
    let a = make(
        src_dir.path(),
        "o1.jpg",
        &jpeg_with_exif(&red_blue(64, 32), 1, "2024:01:01 00:00:00"),
    );
    let o = create_thumbnails(&no_tools(), &a, 10, cache.path(), &opts()).unwrap();
    let img = load(&o.large);
    assert_eq!(img.dimensions(), (64, 32));
    assert!(is_red(img.get_pixel(6, 16)));
    // 방향 3 (180도): 왼쪽이 파랑
    let b = make(
        src_dir.path(),
        "o3.jpg",
        &jpeg_with_exif(&red_blue(64, 32), 3, "2024:01:01 00:00:00"),
    );
    let o = create_thumbnails(&no_tools(), &b, 11, cache.path(), &opts()).unwrap();
    assert!(is_blue(load(&o.large).get_pixel(6, 16)));
    // 방향 8 (반시계 90도): 세로로, 위쪽이 파랑
    let c = make(
        src_dir.path(),
        "o8.jpg",
        &jpeg_with_exif(&red_blue(64, 32), 8, "2024:01:01 00:00:00"),
    );
    let o = create_thumbnails(&no_tools(), &c, 12, cache.path(), &opts()).unwrap();
    let img = load(&o.large);
    assert_eq!(img.dimensions(), (32, 64));
    assert!(is_blue(img.get_pixel(16, 6)));
    // EXIF 없는 PNG: 기본 방향 1, 촬영일 없음
    let png = make(
        src_dir.path(),
        "plain.png",
        &encode(&red_blue(40, 20), ImageFormat::Png),
    );
    let m = read_image_meta(&png).unwrap();
    assert_eq!((m.orientation, m.taken_at.clone()), (1, None));
    // 잘못된 방향 값(9)은 1 로 취급, 깨진 날짜는 None
    let bad = make(
        src_dir.path(),
        "bad.jpg",
        &jpeg_with_exif(&red_blue(8, 8), 9, "garbage-date-string"),
    );
    let m = read_image_meta(&bad).unwrap();
    assert_eq!((m.orientation, m.taken_at), (1, None));
}

#[test]
fn t_media_05_long_edge_sizes_320_and_1024() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let png = make(
        src_dir.path(),
        "wide.png",
        &encode(&red_blue(2000, 1000), ImageFormat::Png),
    );
    let out = create_thumbnails(&no_tools(), &png, 3, cache.path(), &opts()).unwrap();
    assert_eq!(load(&out.large).dimensions(), (1024, 512));
    assert_eq!(load(&out.small).dimensions(), (SMALL_EDGE, 160));
    assert_eq!(out.small.extension().unwrap(), "webp");
}

// ---------------------------------------------------------------- T-media-06

#[test]
fn t_media_06_one_pixel_not_upscaled() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let p = make(
        src_dir.path(),
        "one.png",
        &encode(
            &RgbImage::from_pixel(1, 1, Rgb([9, 8, 7])),
            ImageFormat::Png,
        ),
    );
    let out = create_thumbnails(&no_tools(), &p, 1, cache.path(), &opts()).unwrap();
    assert_eq!(load(&out.large).dimensions(), (1, 1));
    assert_eq!(load(&out.small).dimensions(), (1, 1));
}

#[test]
fn t_media_06_transparent_png_keeps_alpha() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let img = RgbaImage::from_pixel(16, 16, Rgba([10, 200, 30, 0]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).unwrap();
    let p = make(src_dir.path(), "alpha.png", buf.get_ref());
    let out = create_thumbnails(&no_tools(), &p, 1, cache.path(), &opts()).unwrap();
    assert_eq!(
        load(&out.large).get_pixel(8, 8)[3],
        0,
        "fully transparent stays transparent"
    );
}

/// 최소 PNG: IHDR 가 실제보다 큰 해상도를 주장하는 헤더(+IEND). 디코딩 자체가 시작되면 안 된다.
fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32).to_be_bytes().to_vec();
        let mut crc_in = kind.to_vec();
        crc_in.extend_from_slice(body);
        out.extend_from_slice(&crc_in);
        out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        out
    }
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8비트 RGB
    let mut f = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    f.extend(chunk(b"IHDR", &ihdr));
    f.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    f.extend(chunk(b"IEND", &[]));
    f
}

#[test]
fn t_media_06_huge_dimensions_rejected_before_decode() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // Arrange: 30000x30000 (9억 픽셀, RGB8 로 2.7GB) 를 주장하는 파일 — 데이터는 없다
    let p = make(src_dir.path(), "huge.png", &png_claiming(30000, 30000));
    let started = std::time::Instant::now();
    // Act: FFmpeg 없음(degraded)
    let err = create_thumbnails(&no_tools(), &p, 1, cache.path(), &opts()).unwrap_err();
    // Assert: 디코딩 전에 거부, 빠르게 반환, 산출물 없음
    assert!(
        matches!(
            err,
            MediaError::ImageTooLarge {
                width: 30000,
                height: 30000,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(started.elapsed().as_secs() < 5);
    assert!(!cache.path().join("thumbnails").exists());
}

#[test]
fn t_media_06_pixel_cap_boundary() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let p = make(
        src_dir.path(),
        "b.png",
        &encode(&red_blue(64, 32), ImageFormat::Png),
    );
    let at_limit = ThumbnailOptions {
        max_pixels: 64 * 32,
    };
    assert!(create_thumbnails(&no_tools(), &p, 1, cache.path(), &at_limit).is_ok());
    let below = ThumbnailOptions {
        max_pixels: 64 * 32 - 1,
    };
    assert!(matches!(
        create_thumbnails(&no_tools(), &p, 2, cache.path(), &below).unwrap_err(),
        MediaError::ImageTooLarge { .. }
    ));
}

#[test]
fn t_media_06_corrupt_zero_byte_and_wrong_type_are_asset_errors() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let good = encode(&red_blue(64, 32), ImageFormat::Jpeg);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("zero.jpg", vec![]),
        ("trunc.jpg", good[..good.len() / 2].to_vec()),
        ("garbage.png", b"not an image at all".to_vec()),
    ];
    for (i, (name, bytes)) in cases.into_iter().enumerate() {
        let p = make(src_dir.path(), name, &bytes);
        let r = create_thumbnails(&no_tools(), &p, i as i64, cache.path(), &opts());
        assert!(r.is_err(), "{name} must fail without panic");
    }
    // 문서/음성은 썸네일 대상 아님
    let pdf = make(src_dir.path(), "doc.pdf", b"%PDF-1.4");
    assert!(matches!(
        create_thumbnails(&no_tools(), &pdf, 50, cache.path(), &opts()).unwrap_err(),
        MediaError::Unsupported(_)
    ));
    let wav = make(
        src_dir.path(),
        "a.wav",
        &[b"RIFF".as_slice(), &[0; 4], b"WAVEfmt "].concat(),
    );
    assert!(matches!(
        create_thumbnails(&no_tools(), &wav, 51, cache.path(), &opts()).unwrap_err(),
        MediaError::Unsupported(_)
    ));
    // HEIC 는 FFmpeg 가 없으면 WIC 로 폴백하고, 둘 다 안 되면 두 사유를 함께 담은 Decode 오류 (해당 자산만 실패, P2-3 D7)
    let heic = make(
        src_dir.path(),
        "a.heic",
        &[&[0, 0, 0, 24][..], b"ftypheic", &[0; 8]].concat(),
    );
    let err = create_thumbnails(&no_tools(), &heic, 52, cache.path(), &opts()).unwrap_err();
    assert!(
        matches!(&err, MediaError::Decode(m) if m.contains("ffmpeg") && m.contains("wic fallback")),
        "{err:?}"
    );
}

// ---------------------------------------------------------------- T-media-07

fn preview_jpeg() -> Vec<u8> {
    encode(&red_blue(64, 32), ImageFormat::Jpeg)
}

#[test]
fn t_media_07_raw_preview_extracted_from_dng_structure() {
    let dir = tempfile::tempdir().unwrap();
    let jpeg = preview_jpeg();
    let dng = make(dir.path(), "IMG_0001.dng", &synthetic_dng(&jpeg, 1));
    // 같은 구조를 다른 RAW 확장자로도 (TIFF 구조 RAW 라우팅 확인 — 실제 카메라 파일 아님)
    for ext in ["cr2", "nef", "arw"] {
        let p = dir.path().join(format!("X.{ext}"));
        fs::copy(&dng, &p).unwrap();
        let bytes = extract_raw_preview(&p).unwrap();
        assert_eq!(bytes, jpeg, "{ext}: decoy RAW-data IFD must be skipped");
    }
}

#[test]
fn t_media_07_raw_thumbnail_uses_container_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let dng = make(
        dir.path(),
        "IMG_0002.dng",
        &synthetic_dng(&preview_jpeg(), 6),
    );
    let out = create_thumbnails(&no_tools(), &dng, 7, cache.path(), &opts()).unwrap();
    assert_eq!(out.method, DecodeMethod::RawPreview);
    assert_eq!(
        out.large_size,
        (32, 64),
        "orientation 6 from the RAW container is applied"
    );
    assert!(is_red(load(&out.large).get_pixel(16, 6)));
}

#[test]
fn t_media_07_raw_failures() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // 프리뷰가 없는 평범한 TIFF 를 .dng 로
    let tiff = encode(&red_blue(8, 8), ImageFormat::Tiff);
    let p = make(dir.path(), "plain.dng", &tiff);
    assert!(matches!(
        extract_raw_preview(&p).unwrap_err(),
        MediaError::NoRawPreview
    ));
    assert!(create_thumbnails(&no_tools(), &p, 1, cache.path(), &opts()).is_err());
    // 프리뷰 구간 중간에서 잘린 파일
    let full = synthetic_dng(&preview_jpeg(), 1);
    let cut = make(dir.path(), "cut.dng", &full[..full.len() - 100]);
    assert!(matches!(
        extract_raw_preview(&cut).unwrap_err(),
        MediaError::NoRawPreview
    ));
    // 자기 자신을 가리키는 순환 IFD — 무한 루프 없이 종료
    let mut cyc = b"II*\0".to_vec();
    cyc.extend_from_slice(&8u32.to_le_bytes());
    cyc.extend_from_slice(&1u16.to_le_bytes());
    cyc.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    cyc.extend_from_slice(&8u32.to_le_bytes());
    let c = make(dir.path(), "cycle.dng", &cyc);
    assert!(matches!(
        extract_raw_preview(&c).unwrap_err(),
        MediaError::NoRawPreview
    ));
    // 0바이트 / TIFF 아님
    let z = make(dir.path(), "zero.dng", b"");
    assert!(matches!(
        extract_raw_preview(&z).unwrap_err(),
        MediaError::NoRawPreview
    ));
}

// ---------------------------------------------------------------- T-media-08

fn list_all(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn t_media_08_original_is_untouched_and_outputs_only_in_cache() {
    // Arrange: 원본 폴더와 캐시 폴더를 분리, 원본은 읽기 전용 속성으로 둔다 (쓰기 시도가 있으면 실패하도록)
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let files = [
        make(
            src_dir.path(),
            "a.jpg",
            &jpeg_with_exif(&red_blue(64, 32), 6, "2024:05:01 12:34:56"),
        ),
        make(
            src_dir.path(),
            "b.png",
            &encode(&red_blue(300, 300), ImageFormat::Png),
        ),
        make(src_dir.path(), "c.dng", &synthetic_dng(&preview_jpeg(), 1)),
    ];
    let before: Vec<_> = files.iter().map(|p| snapshot(p)).collect();
    let listing_before = list_all(src_dir.path());
    for p in &files {
        let mut perm = fs::metadata(p).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(p, perm).unwrap();
    }

    // Act
    for (i, p) in files.iter().enumerate() {
        read_image_meta(p).unwrap();
        create_thumbnails(&no_tools(), p, i as i64 + 100, cache.path(), &opts()).unwrap();
    }

    // Assert: 내용·수정 시각 불변, 원본 폴더에 새 파일 없음, 산출물은 캐시 루트 아래 thumbnails/ 에만
    let after: Vec<_> = files.iter().map(|p| snapshot(p)).collect();
    assert_eq!(before, after);
    assert_eq!(listing_before, list_all(src_dir.path()));
    let produced = list_all(cache.path());
    assert_eq!(produced.len(), files.len() * 2);
    assert!(
        produced
            .iter()
            .all(|p| p.starts_with(cache.path().join("thumbnails")))
    );
    for p in &files {
        let mut perm = fs::metadata(p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(p, perm).unwrap();
    }
}

// ---------------------------------------------------------------- T-media-11

#[test]
fn t_media_11_sharded_path_layout() {
    let root = Path::new("C:/MAMData/cache");
    // 1234567 = 0x12D687 -> 하위 바이트 0x87, 다음 바이트 0xD6
    let p = thumbnail_path(root, 1_234_567, ThumbSize::Small);
    assert_eq!(p, root.join("thumbnails/320/87/d6/1234567.webp"));
    let l = thumbnail_path(root, 1_234_567, ThumbSize::Large);
    assert_eq!(l, root.join("thumbnails/1024/87/d6/1234567.webp"));
    // 작은 ID 도 0 채움
    assert_eq!(
        thumbnail_path(root, 5, ThumbSize::Small),
        root.join("thumbnails/320/05/00/5.webp")
    );
    // 연속 ID 는 서로 다른 샤드로 퍼진다
    let shards: std::collections::HashSet<_> = (1..=256)
        .map(|i| {
            thumbnail_path(root, i, ThumbSize::Small)
                .parent()
                .unwrap()
                .to_path_buf()
        })
        .collect();
    assert_eq!(shards.len(), 256);
    // 음수 ID 도 패닉 없이 경로 생성
    let _ = thumbnail_path(root, -1, ThumbSize::Small);
}

#[test]
fn t_media_11_regeneration_overwrites_without_leftovers() {
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let red = make(
        src_dir.path(),
        "red.png",
        &encode(
            &RgbImage::from_pixel(50, 50, Rgb([255, 0, 0])),
            ImageFormat::Png,
        ),
    );
    let blue = make(
        src_dir.path(),
        "blue.png",
        &encode(
            &RgbImage::from_pixel(50, 50, Rgb([0, 0, 255])),
            ImageFormat::Png,
        ),
    );

    let first = create_thumbnails(&no_tools(), &red, 42, cache.path(), &opts()).unwrap();
    assert!(is_red(load(&first.large).get_pixel(5, 5)));
    // 같은 asset_id 로 다른 내용을 재생성 -> 같은 경로가 새 내용으로 교체
    let second = create_thumbnails(&no_tools(), &blue, 42, cache.path(), &opts()).unwrap();
    assert_eq!(first.large, second.large);
    assert!(is_blue(load(&second.large).get_pixel(5, 5)));
    // 임시 파일이 남지 않는다
    let all = list_all(cache.path());
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(all.iter().all(|p| p.extension().unwrap() == "webp"));
    // 다른 asset 은 서로 다른 파일
    let other = create_thumbnails(&no_tools(), &red, 43, cache.path(), &opts()).unwrap();
    assert_ne!(other.large, second.large);
    assert_eq!(list_all(cache.path()).len(), 4);
}

// ---------------------------------------------------------------- T-media-06 (HEIC)

#[test]
fn t_media_06_heic_without_decoder_fails_per_asset() {
    // 단일 아이템 HEIC (x265 인트라 1프레임을 직접 HEIF 로 조립한 3KB 픽스처)
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let heic = copy_fixture("tiny.heic", dir.path());
    // FFmpeg 없음: 판별은 HEIF, 디코더 둘(FFmpeg·WIC)이 모두 안 되면 자산 단위 실패 (WIC HEIF·HEVC 코덱이 있는 PC 에서는 성공)
    let d = mam_media::detect(&heic).unwrap();
    assert_eq!(d.container, Some(mam_media::Container::Heif));
    match create_thumbnails(&no_tools(), &heic, 1, cache.path(), &opts()) {
        Ok(o) => assert_eq!(o.method, DecodeMethod::Wic),
        Err(e) => assert!(
            matches!(&e, MediaError::Decode(m) if m.contains("ffmpeg") && m.contains("wic fallback")),
            "{e:?}"
        ),
    }
    // 실제 FFmpeg: 6.0 은 HEIF 아이템을 열지 못한다(알려진 한계) - 성공하거나 ToolFailed 여야 하고 패닉·멈춤 없음
    match real_tools() {
        None => skip(
            "t_media_06_heic_without_decoder_fails_per_asset",
            "ffmpeg not found (real-ffmpeg half skipped)",
        ),
        Some(tools) => match create_thumbnails(&tools, &heic, 1, cache.path(), &opts()) {
            Ok(o) => eprintln!("HEIC decoded by this ffmpeg via {:?}", o.method),
            Err(e) => {
                eprintln!("HEIC not decodable by this ffmpeg: {e}");
                assert!(
                    matches!(e, MediaError::ToolFailed { .. })
                        || matches!(&e, MediaError::Decode(m) if m.contains("wic fallback")),
                    "{e:?}"
                );
            }
        },
    }
}

#[test]
fn t_media_05_webp_is_lossy_so_photo_thumbnails_stay_small() {
    // 질감이 있는 사진 흉내 이미지: 무손실 WebP 라면 1024x768 RGB(2.3MB) 의 절반 이상이 되지만 손실은 훨씬 작다
    let src_dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let textured = RgbImage::from_fn(1600, 1200, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let v = ((fx * 0.07).sin() * 60.0
            + (fy * 0.05).cos() * 60.0
            + ((x * 31 + y * 17) % 13) as f32 * 4.0) as i32;
        Rgb([
            (128 + v).clamp(0, 255) as u8,
            (100 + v / 2).clamp(0, 255) as u8,
            (90 - v / 3).clamp(0, 255) as u8,
        ])
    });
    let p = make(
        src_dir.path(),
        "photo.png",
        &encode(&textured, ImageFormat::Png),
    );
    let out = create_thumbnails(&no_tools(), &p, 1, cache.path(), &opts()).unwrap();
    let (large, small) = (
        fs::metadata(&out.large).unwrap().len(),
        fs::metadata(&out.small).unwrap().len(),
    );
    let raw_large = 1024 * 768 * 3;
    assert!(large < raw_large / 5, "large webp {large} bytes");
    assert!(small < large);
    assert_eq!(&fs::read(&out.large).unwrap()[..4], b"RIFF");
}
