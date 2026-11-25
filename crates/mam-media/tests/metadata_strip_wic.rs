//! T-proc-media-*: 통합 메타데이터 추출, 영상 프레임 스트립, WIC 디코딩 (P2-3).
//!
//! 분류: Happy (메타·스트립·WIC) / Boundary (길이 모름, 크기 상한) / External (도구 없음·프레임 실패) / Validation (잘못된 파일).
//! FFmpeg 는 스텁 실행기로 대체한다 (결정적). WIC 는 Windows 내장 PNG/JPEG 코덱으로 연결 자체를 검증하고,
//! HEIF 코덱이 설치된 PC 에서만 실제 HEIC 를 디코딩한다.

mod common;

use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use image::ImageFormat;
use mam_media::error::MediaError;
use mam_media::strip::{STRIP_FRAMES, create_frame_strip, strip_path};
use mam_media::tool::Tool;
use mam_media::{MediaKind, Playback, decode_with_wic, extract_metadata};

const PROBE_MP4: &str = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
    "avg_frame_rate":"25/1","side_data_list":[{"rotation":-90}]},{"codec_type":"audio","codec_name":"aac"}],
    "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.5","bit_rate":"4000000",
    "tags":{"creation_time":"2023-12-31T23:00:00Z"}}}"#;

fn png(w: u32, h: u32) -> Vec<u8> {
    encode(&red_blue(w, h), ImageFormat::Png)
}

#[test]
fn t_proc_media_01_image_metadata_applies_orientation_and_reads_exif() {
    // Arrange: 64x32 JPEG, EXIF 방향 6 (표시 크기는 32x64)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rot.jpg");
    fs::write(
        &path,
        jpeg_with_exif(&red_blue(64, 32), 6, "2024:05:01 12:34:56"),
    )
    .unwrap();

    // Act
    let m = extract_metadata(&no_tools(), &path, MediaKind::Image).unwrap();

    // Assert
    assert_eq!((m.width, m.height), (Some(32), Some(64)));
    assert_eq!(m.orientation, Some(6));
    assert_eq!(m.codec.as_deref(), Some("jpeg"));
    assert_eq!(m.captured_at_ms, Some(1_714_566_896_000));
    assert_eq!(m.playback, Playback::Playable);
    assert!(m.duration_ms.is_none() && m.audio_codec.is_none());
}

#[test]
fn t_proc_media_02_video_metadata_uses_ffprobe_with_rotation_and_playback_verdict() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.mp4");
    fs::write(&path, b"stub").unwrap();
    let tools = stub_tools(|tool, _| match tool {
        Tool::Ffprobe => Ok(ok_output(PROBE_MP4.as_bytes())),
        Tool::Ffmpeg => Err(MediaError::InvalidOutput("unused".into())),
    });

    // Act
    let m = extract_metadata(&tools, &path, MediaKind::Video).unwrap();

    // Assert: 회전 90 -> 표시 크기 가로·세로 교환
    assert_eq!((m.width, m.height), (Some(1080), Some(1920)));
    assert_eq!(m.duration_ms, Some(12_500));
    assert_eq!(
        (m.codec.as_deref(), m.audio_codec.as_deref()),
        (Some("h264"), Some("aac"))
    );
    assert_eq!((m.fps, m.bitrate), (Some(25.0), Some(4_000_000)));
    assert_eq!(m.captured_at_ms, Some(1_704_063_600_000));
    assert_eq!(m.playback, Playback::Playable);
    assert_eq!(m.container.as_deref(), Some("mov,mp4,m4a,3gp,3g2,mj2"));
}

#[test]
fn t_proc_media_03_missing_tools_and_broken_files_are_errors_not_panics() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("v.mp4");
    fs::write(&video, b"stub").unwrap();
    let broken = dir.path().join("broken.png");
    fs::write(&broken, b"not an image").unwrap();

    // Act / Assert: 영상은 ffprobe 필수
    assert!(matches!(
        extract_metadata(&no_tools(), &video, MediaKind::Video),
        Err(MediaError::ToolNotFound { .. })
    ));
    // 깨진 이미지는 헤더를 읽지 못해도 오류가 아니라 크기 없음 (썸네일 단계가 실패로 기록한다)
    let m = extract_metadata(&no_tools(), &broken, MediaKind::Image).unwrap();
    assert_eq!((m.width, m.height), (None, None));
    // 없는 파일은 I/O 오류
    assert!(matches!(
        extract_metadata(&no_tools(), &dir.path().join("none.png"), MediaKind::Image),
        Err(MediaError::Io { .. })
    ));
}

fn frame_stub(calls: &'static AtomicUsize, fail_at: Option<usize>) -> mam_media::MediaTools {
    stub_tools(move |tool, _| match tool {
        Tool::Ffprobe => Ok(ok_output(PROBE_MP4.as_bytes())),
        Tool::Ffmpeg => {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            if fail_at == Some(n) {
                return Ok(ok_output(b"")); // 프레임 없음
            }
            Ok(ok_output(&png(80, 45)))
        }
    })
}

#[test]
fn t_proc_media_04_frame_strip_has_five_equal_cells_and_is_written_under_previews() {
    // Arrange
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let cache = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let video = src.path().join("v.mp4");
    fs::write(&video, b"stub").unwrap();
    let before = snapshot(&video);

    // Act
    let out = create_frame_strip(
        &frame_stub(&CALLS, None),
        &video,
        42,
        cache.path(),
        Some(10.0),
    )
    .unwrap();

    // Assert: 5칸 가로 스트립 WebP, 샤딩 경로, 칸 크기 일정
    assert_eq!(CALLS.load(Ordering::SeqCst), STRIP_FRAMES);
    assert_eq!(out.path, strip_path(cache.path(), 42));
    assert!(out.path.starts_with(cache.path().join("previews")));
    let img = image::open(&out.path).unwrap();
    assert_eq!(
        (img.width(), img.height()),
        (out.cell.0 * STRIP_FRAMES as u32, out.cell.1)
    );
    // 왼쪽 빨강/오른쪽 파랑 프레임 -> 첫 칸의 왼쪽 가장자리는 빨강
    let rgb = img.to_rgb8();
    let left = rgb.get_pixel(2, out.cell.1 / 2);
    assert!(left[0] > 180 && left[2] < 90, "{left:?}");
    // 원본 불변, 캐시 밖에 쓰지 않음
    assert_eq!(snapshot(&video), before);
    assert_eq!(fs::read_dir(src.path()).unwrap().count(), 1);
}

#[test]
fn t_proc_media_05_frame_strip_boundaries_unknown_duration_and_partial_failures() {
    // Arrange
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let cache = tempfile::tempdir().unwrap();
    let video = cache.path().join("v.mp4");
    fs::write(&video, b"stub").unwrap();

    // Act / Assert: 길이를 모르면 위치를 정할 수 없다 -> Unsupported (프레임 추출을 시도하지 않는다)
    let tools = frame_stub(&CALLS, None);
    for d in [None, Some(0.0), Some(-1.0), Some(f64::NAN)] {
        assert!(matches!(
            create_frame_strip(&tools, &video, 1, cache.path(), d),
            Err(MediaError::Unsupported(_))
        ));
    }
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);

    // 중간 프레임 하나가 비어 있어도 앞 칸으로 채워 완성한다
    let ok = create_frame_strip(
        &frame_stub(&CALLS, Some(2)),
        &video,
        2,
        cache.path(),
        Some(10.0),
    );
    assert!(ok.is_ok(), "{ok:?}");

    // 도구가 없으면 즉시 실패 (파일을 남기지 않는다)
    let err = create_frame_strip(&no_tools(), &video, 3, cache.path(), Some(10.0)).unwrap_err();
    assert!(matches!(err, MediaError::ToolNotFound { .. }));
    assert!(!strip_path(cache.path(), 3).exists());
}

#[cfg(windows)]
#[test]
fn t_proc_media_06_wic_decodes_builtin_formats_and_rejects_bad_input() {
    // Arrange: Windows 내장 코덱으로 WIC 연결(COM·팩토리·변환) 자체를 검증한다
    let dir = tempfile::tempdir().unwrap();
    let ok = dir.path().join("a.png");
    fs::write(&ok, png(40, 20)).unwrap();
    let junk = dir.path().join("junk.heic");
    fs::write(&junk, b"definitely not an image").unwrap();

    // Act
    let img = decode_with_wic(&ok, 1_000_000).unwrap();

    // Assert: 크기·색 (왼쪽 빨강, 오른쪽 파랑)
    assert_eq!((img.width(), img.height()), (40, 20));
    let rgba = img.to_rgba8();
    assert!(rgba.get_pixel(2, 10)[0] > 200 && rgba.get_pixel(2, 10)[2] < 60);
    assert!(rgba.get_pixel(37, 10)[2] > 200 && rgba.get_pixel(37, 10)[0] < 60);
    // 크기 상한을 넘으면 픽셀을 읽기 전에 거부
    assert!(matches!(
        decode_with_wic(&ok, 100),
        Err(MediaError::ImageTooLarge { .. })
    ));
    // 코덱이 없거나 손상된 파일, 없는 파일은 오류 (패닉·행 없음)
    assert!(matches!(
        decode_with_wic(&junk, 1_000_000),
        Err(MediaError::Decode(_))
    ));
    assert!(decode_with_wic(&dir.path().join("none.png"), 1_000_000).is_err());
    // 여러 번·여러 스레드에서 호출해도 COM 초기화가 균형을 유지한다
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let p = ok.clone();
            std::thread::spawn(move || decode_with_wic(&p, 1_000_000).is_ok())
        })
        .collect();
    assert!(handles.into_iter().all(|h| h.join().unwrap()));
}

#[test]
fn t_proc_media_07_wic_decodes_real_heic_when_the_heif_codec_is_installed() {
    // Arrange: 환경 의존 - HEIF 확장이 없으면 디코딩 실패가 정상이다 (T-proc-10: 실패 기록 경로)
    let heic = fixtures_dir().join("tiny.heic");

    // Act
    let result = decode_with_wic(&heic, 64_000_000);

    // Assert
    match result {
        Ok(img) => assert!(img.width() > 0 && img.height() > 0),
        Err(e) => {
            skip("t_proc_media_07", "HEIF codec not installed");
            assert!(matches!(e, MediaError::Decode(_)), "{e:?}");
        }
    }
}
