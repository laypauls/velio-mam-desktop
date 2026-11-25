//! T-media-09, 10: 영상 대표 프레임·썸네일, WebView2 재생 판정표.
//!
//! 분류: Happy / External / Business rule. 실제 ffmpeg 가 필요한 테스트는 없으면 사유를 출력하고 건너뛴다.

mod common;

use std::fs;
use std::sync::{Arc, Mutex};

use common::*;
use image::ImageFormat;
use mam_media::error::MediaError;
use mam_media::frames::representative_positions;
use mam_media::playback::{Playback, playback_of, webview_playback};
use mam_media::probe::parse_probe_json;
use mam_media::thumbnail::{DecodeMethod, ThumbnailOptions, create_thumbnails};
use mam_media::tool::Tool;

// ---------------------------------------------------------------- T-media-09

#[test]
fn t_media_09_representative_positions() {
    assert_eq!(representative_positions(Some(10.0)), vec![1.0, 5.0, 9.0]);
    // 길이를 모르거나 0/비정상이면 첫 프레임
    for d in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        assert_eq!(representative_positions(d), vec![0.0], "{d:?}");
    }
}

#[test]
fn t_media_09_video_thumbnail_seeks_10_percent_then_falls_back_to_start() {
    // Arrange: 길이 10초 -> 1.0초 지점 시도, 프레임이 없으면 0초로 재시도하는 스텁
    let json = fs::read(fixtures_dir().join("probe_h264.mp4.json")).unwrap();
    let json = String::from_utf8(json)
        .unwrap()
        .replace("\"duration\": \"1.000000\"", "\"duration\": \"10.0\"");
    let png = encode(&red_blue(64, 48), ImageFormat::Png);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let s2 = Arc::clone(&seen);
    let tools = stub_tools(move |tool, args| match tool {
        Tool::Ffprobe => Ok(ok_output(json.as_bytes())),
        Tool::Ffmpeg => {
            let ss = args
                .windows(2)
                .find(|w| w[0] == "-ss")
                .map(|w| w[1].to_string_lossy().into_owned())
                .unwrap();
            s2.lock().unwrap().push(ss.clone());
            if ss == "1.000" {
                Ok(ok_output(b"")) // 탐색 지점에 프레임 없음
            } else {
                Ok(ok_output(&png))
            }
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mp4 = copy_fixture("tiny.mp4", dir.path());

    // Act
    let out =
        create_thumbnails(&tools, &mp4, 1, cache.path(), &ThumbnailOptions::default()).unwrap();

    // Assert
    assert_eq!(out.method, DecodeMethod::VideoFrame);
    assert_eq!(
        *seen.lock().unwrap(),
        vec!["1.000".to_string(), "0.000".to_string()]
    );
}

#[test]
fn t_media_09_ffmpeg_failure_is_asset_error_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mp4 = copy_fixture("tiny.mp4", dir.path());
    let tools = stub_tools(|tool, _| match tool {
        Tool::Ffprobe => Err(MediaError::InvalidOutput("probe broken".into())),
        Tool::Ffmpeg => Ok(fail_output(1, "moov atom not found")),
    });
    let err =
        create_thumbnails(&tools, &mp4, 1, cache.path(), &ThumbnailOptions::default()).unwrap_err();
    assert!(matches!(err, MediaError::ToolFailed { .. }), "{err:?}");
    assert!(!cache.path().join("thumbnails").exists());
    // ffmpeg 가 없으면 영상 썸네일만 실패한다 (degraded)
    let degraded =
        mam_media::MediaTools::with_paths(None, None, Arc::new(mam_media::ProcessRunner));
    let err = create_thumbnails(
        &degraded,
        &mp4,
        1,
        cache.path(),
        &ThumbnailOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, MediaError::ToolNotFound { .. }), "{err:?}");
}

#[test]
fn t_media_09_real_ffmpeg_frame_and_thumbnail() {
    let Some(tools) = real_tools() else {
        skip(
            "t_media_09_real_ffmpeg_frame_and_thumbnail",
            "ffmpeg/ffprobe not found on PATH",
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    for name in ["tiny.mp4", "tiny.webm"] {
        let src = copy_fixture(name, dir.path());
        let before = snapshot(&src);
        let probe = tools.probe(&src).unwrap();
        let frames = representative_positions(probe.duration_secs);
        assert_eq!(frames.len(), 3);
        // 대표 프레임 3장 모두 추출 가능
        for t in frames {
            let f = tools.extract_frame(&src, Some(t), 1024).unwrap();
            assert_eq!((f.width(), f.height()), (64, 48), "{name} @ {t}");
        }
        let out =
            create_thumbnails(&tools, &src, 9, cache.path(), &ThumbnailOptions::default()).unwrap();
        assert_eq!(out.method, DecodeMethod::VideoFrame);
        assert_eq!(out.large_size, (64, 48));
        assert!(out.small.is_file() && out.large.is_file());
        assert_eq!(before, snapshot(&src), "original unchanged");
    }
}

#[test]
fn t_media_09_real_ffmpeg_rotation_and_corrupt() {
    let Some(tools) = real_tools() else {
        skip(
            "t_media_09_real_ffmpeg_rotation_and_corrupt",
            "ffmpeg/ffprobe not found on PATH",
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // 회전 메타데이터(90도) 영상: 표시 크기 48x64 로 썸네일이 만들어져야 한다
    let rot = dir.path().join("rot.mp4");
    let st = std::process::Command::new(tools_path("ffmpeg"))
        .args(["-v", "error", "-y", "-display_rotation", "90", "-i"])
        .arg(fixtures_dir().join("tiny.mp4"))
        .args(["-c", "copy"])
        .arg(&rot)
        .status();
    if st.map(|s| !s.success()).unwrap_or(true) {
        skip(
            "t_media_09_real_ffmpeg_rotation_and_corrupt",
            "could not build rotated sample",
        );
        return;
    }
    let out =
        create_thumbnails(&tools, &rot, 1, cache.path(), &ThumbnailOptions::default()).unwrap();
    assert_eq!(out.large_size, (48, 64));
    // 앞부분이 잘린/손상된 영상은 자산 오류
    let bad = dir.path().join("bad.mp4");
    fs::write(
        &bad,
        &fs::read(fixtures_dir().join("tiny.mp4")).unwrap()[..600],
    )
    .unwrap();
    assert!(
        create_thumbnails(&tools, &bad, 2, cache.path(), &ThumbnailOptions::default()).is_err()
    );
}

fn tools_path(name: &str) -> std::path::PathBuf {
    use mam_media::tool::{Tool as T, ToolConfig, resolve_tool};
    let t = if name == "ffmpeg" {
        T::Ffmpeg
    } else {
        T::Ffprobe
    };
    resolve_tool(t, &ToolConfig::default()).unwrap()
}

// ---------------------------------------------------------------- T-media-10

#[test]
#[allow(clippy::type_complexity)]
fn t_media_10_playback_table() {
    use Playback::*;
    let mp4 = "mov,mp4,m4a,3gp,3g2,mj2";
    let mkv = "matroska,webm";
    let cases: Vec<(&str, &str, Option<&str>, Option<&str>, Playback)> = vec![
        ("mp4/h264+aac", mp4, Some("h264"), Some("aac"), Playable),
        ("mp4/h264 only", mp4, Some("h264"), None, Playable),
        ("webm/vp9+opus", mkv, Some("vp9"), Some("opus"), Playable),
        (
            "webm/vp8+vorbis",
            mkv,
            Some("vp8"),
            Some("vorbis"),
            Playable,
        ),
        ("mp4/av1", mp4, Some("av1"), Some("aac"), Playable),
        ("mp4/hevc", mp4, Some("hevc"), Some("aac"), Conditional),
        ("mkv/h264+aac", mkv, Some("h264"), Some("aac"), Conditional),
        ("mxf/mpeg2", "mxf", Some("mpeg2video"), None, NotPlayable),
        ("wmv", "asf", Some("wmv2"), Some("wmav2"), NotPlayable),
        ("avi/mpeg4", "avi", Some("mpeg4"), Some("mp3"), NotPlayable),
        ("avi/h264", "avi", Some("h264"), Some("aac"), NotPlayable),
        (
            "mpegts/h264",
            "mpegts",
            Some("h264"),
            Some("aac"),
            NotPlayable,
        ),
        (
            "mpeg-ps",
            "mpeg",
            Some("mpeg2video"),
            Some("mp2"),
            NotPlayable,
        ),
        (
            "mp4/prores",
            mp4,
            Some("prores"),
            Some("pcm_s16le"),
            NotPlayable,
        ),
        ("mp4/h264+ac3", mp4, Some("h264"), Some("ac3"), NotPlayable),
        ("mp3", "mp3", None, Some("mp3"), Playable),
        ("wav", "wav", None, Some("pcm_s16le"), Playable),
        ("flac", "flac", None, Some("flac"), Playable),
        ("ogg/vorbis", "ogg", None, Some("vorbis"), Playable),
        ("ogg/opus", "ogg", None, Some("opus"), Playable),
        ("m4a/aac", mp4, None, Some("aac"), Playable),
        (
            "ogg/theora",
            "ogg",
            Some("theora"),
            Some("vorbis"),
            NotPlayable,
        ),
        ("wma", "asf", None, Some("wmav2"), NotPlayable),
        ("aiff", "aiff", None, Some("pcm_s16be"), NotPlayable),
    ];
    for (label, container, v, a, want) in cases {
        let got = webview_playback(container, v, a);
        assert_eq!(got.playback, want, "{label}: {}", got.reason);
    }
}

#[test]
fn t_media_10_playback_from_probe_fixtures() {
    let verdict = |name: &str| {
        let json = fs::read(fixtures_dir().join(name)).unwrap();
        playback_of(&parse_probe_json(&json).unwrap()).playback
    };
    assert_eq!(verdict("probe_h264.mp4.json"), Playback::Playable);
    assert_eq!(verdict("probe_rot.mp4.json"), Playback::Playable);
    assert_eq!(verdict("probe_vp9.webm.json"), Playback::Playable);
    assert_eq!(verdict("probe_a.mp3.json"), Playback::Playable);
    assert_eq!(verdict("probe_mpeg2.mxf.json"), Playback::NotPlayable);
    assert_eq!(verdict("probe_wmv2.wmv.json"), Playback::NotPlayable);
}
