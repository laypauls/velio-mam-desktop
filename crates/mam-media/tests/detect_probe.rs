//! T-media-01 ~ 04: 형식 판별, ffprobe JSON 파싱, 외부 프로세스 실패 경로, 경계 경로.
//!
//! 분류: Happy / External / Boundary. 실제 ffprobe 가 필요한 테스트는 없으면 사유를 출력하고 건너뛴다.

mod common;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use image::ImageFormat;
use mam_media::error::MediaError;
use mam_media::format::{Container, MediaKind, classify, detect};
use mam_media::probe::parse_probe_json;
use mam_media::tool::{
    CommandRunner, MediaTools, ProcessRunner, RunOptions, Tool, ToolConfig, resolve_tool,
};

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

// ---------------------------------------------------------------- T-media-01

#[test]
fn t_media_01_detect_by_extension_and_magic() {
    // Arrange: 확장자와 내용이 일치하는 파일들
    let dir = tempfile::tempdir().unwrap();
    let png = encode(&red_blue(8, 8), ImageFormat::Png);
    let jpg = encode(&red_blue(8, 8), ImageFormat::Jpeg);
    let mp4 = fs::read(fixtures_dir().join("tiny.mp4")).unwrap();
    let webm = fs::read(fixtures_dir().join("tiny.webm")).unwrap();
    let cases: Vec<(&str, &[u8], Option<MediaKind>)> = vec![
        ("a.png", &png, Some(MediaKind::Image)),
        ("a.JPG", &jpg, Some(MediaKind::Image)), // 대소문자
        ("a.mp4", &mp4, Some(MediaKind::Video)),
        ("a.webm", &webm, Some(MediaKind::Video)),
        ("a.txt", b"hello world", None),
        ("a.pdf", b"%PDF-1.7 ...", None),
        ("a.docx", b"PK\x03\x04....", None),
        ("noext", b"hello", None),
    ];
    for (name, bytes, want) in cases {
        let p = dir.path().join(name);
        fs::write(&p, bytes).unwrap();
        // Act
        let d = detect(&p).unwrap();
        // Assert
        assert_eq!(d.kind, want, "{name}");
        assert!(!d.mismatch, "{name} should not be a mismatch");
    }
    // 내용을 모르는 확장자 지원 파일(예: MXF 변형)은 확장자를 따른다
    assert_eq!(classify("mxf", b"\0\0\0\0").kind, Some(MediaKind::Video));
}

#[test]
fn t_media_01_magic_bytes_per_container() {
    let wav = [b"RIFF".as_slice(), &[0; 4], b"WAVEfmt "].concat();
    let avi = [b"RIFF".as_slice(), &[0; 4], b"AVI LIST"].concat();
    let webp = [b"RIFF".as_slice(), &[0; 4], b"WEBPVP8 "].concat();
    let heic = [&[0, 0, 0, 24][..], b"ftypheic", &[0; 8]].concat();
    let avif = [&[0, 0, 0, 24][..], b"ftypavif", &[0; 8]].concat();
    let mov = [&[0, 0, 0, 20][..], b"ftypqt  ", &[0; 8]].concat();
    let aiff = [b"FORM".as_slice(), &[0; 4], b"AIFF"].concat();
    let mxf = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01,
    ];
    let mut ts = vec![0u8; 188 * 3];
    for i in 0..3 {
        ts[i * 188] = 0x47;
    }
    let mut m2ts = vec![0u8; 4 + 192 * 3];
    for i in 0..3 {
        m2ts[4 + i * 192] = 0x47;
    }
    let cases: Vec<(&str, &[u8], Container)> = vec![
        ("wav", &wav, Container::Wav),
        ("avi", &avi, Container::Avi),
        ("webp", &webp, Container::Webp),
        ("heic", &heic, Container::Heif),
        ("avif", &avif, Container::Avif),
        ("mov", &mov, Container::IsoBmff),
        ("mxf", &mxf, Container::Mxf),
        ("mts", &ts, Container::MpegTs),
        ("m2ts", &m2ts, Container::MpegTs),
        ("mkv", &[0x1A, 0x45, 0xDF, 0xA3, 0], Container::Matroska),
        (
            "wmv",
            &[0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11],
            Container::Asf,
        ),
        ("mp3", b"ID3\x03\0\0", Container::Mp3),
        ("mp3", &[0xFF, 0xFB, 0x90, 0x00], Container::Mp3),
        ("aac", &[0xFF, 0xF1, 0x50, 0x80], Container::AacAdts),
        ("flac", b"fLaC\0", Container::Flac),
        ("ogg", b"OggS\0", Container::Ogg),
        ("aiff", &aiff, Container::Aiff),
        ("tif", b"II*\0\x08\0\0\0", Container::Tiff),
        ("dng", b"MM\0*\0\0\0\x08", Container::Tiff),
    ];
    for (ext, bytes, want) in cases {
        assert_eq!(classify(ext, bytes).container, Some(want), "{ext}");
    }
}

#[test]
fn t_media_01_disguised_extension() {
    // PDF 가 .jpg 로 위장 -> 미디어 아님 + 불일치
    let d = classify("jpg", b"%PDF-1.4");
    assert_eq!((d.kind, d.mismatch), (None, true));
    // MP4 가 .jpg 로 위장 -> 영상으로 취급 + 불일치
    let mp4 = [&[0, 0, 0, 24][..], b"ftypisom", &[0; 8]].concat();
    let d = classify("jpg", &mp4);
    assert_eq!((d.kind, d.mismatch), (Some(MediaKind::Video), true));
    // PNG 내용인데 확장자가 .mp3 -> 이미지 + 불일치
    let d = classify("mp3", &[0x89, b'P', b'N', b'G']);
    assert_eq!((d.kind, d.mismatch), (Some(MediaKind::Image), true));
    // 확장자 없는 PNG -> 이미지 (불일치 아님이 아니라 확장자 종류 없음 = 불일치로 표시)
    let d = classify("", &[0x89, b'P', b'N', b'G']);
    assert_eq!(d.kind, Some(MediaKind::Image));
    // 영상/음성 겸용 컨테이너: 음성 확장자는 음성으로
    let m4a = [&[0, 0, 0, 24][..], b"ftypM4A ", &[0; 8]].concat();
    assert_eq!(classify("m4a", &m4a).kind, Some(MediaKind::Audio));
}

#[test]
fn t_media_01_missing_file_is_io_error() {
    let err = detect(&PathBuf::from("Z:/definitely/not/here.jpg")).unwrap_err();
    assert!(matches!(err, MediaError::Io { .. }));
}

// ---------------------------------------------------------------- T-media-02

fn fixture_json(name: &str) -> Vec<u8> {
    fs::read(fixtures_dir().join(name)).unwrap()
}

#[test]
fn t_media_02_parse_h264_mp4() {
    let p = parse_probe_json(&fixture_json("probe_h264.mp4.json")).unwrap();
    assert!(p.container.contains("mp4"));
    assert!((p.duration_secs.unwrap() - 1.0).abs() < 0.2);
    let v = p.video.unwrap();
    assert_eq!(
        (v.codec.as_str(), v.width, v.height, v.rotation),
        ("h264", 64, 48, 0)
    );
    assert_eq!(v.fps, Some(25.0));
    assert_eq!(p.audio.unwrap().codec, "aac");
}

#[test]
fn t_media_02_parse_rotation() {
    let p = parse_probe_json(&fixture_json("probe_rot.mp4.json")).unwrap();
    let v = p.video.unwrap();
    assert_eq!(v.rotation, 90);
    assert_eq!(v.display_size(), (48, 64));
}

#[test]
fn t_media_02_parse_webm_mxf_wmv_mp3() {
    let webm = parse_probe_json(&fixture_json("probe_vp9.webm.json")).unwrap();
    assert!(webm.container.contains("webm"));
    assert_eq!(webm.video.unwrap().codec, "vp9");

    let mxf = parse_probe_json(&fixture_json("probe_mpeg2.mxf.json")).unwrap();
    assert_eq!(mxf.container, "mxf");
    assert_eq!(mxf.video.unwrap().codec, "mpeg2video");

    let wmv = parse_probe_json(&fixture_json("probe_wmv2.wmv.json")).unwrap();
    assert_eq!(wmv.container, "asf");
    assert_eq!(wmv.video.unwrap().codec, "wmv2");

    let mp3 = parse_probe_json(&fixture_json("probe_a.mp3.json")).unwrap();
    assert!(mp3.video.is_none());
    let a = mp3.audio.unwrap();
    assert_eq!(a.codec, "mp3");
    assert_eq!(a.sample_rate, Some(44100));
}

#[test]
fn t_media_02_creation_time_negative_rotation_and_cover_art() {
    let json = br#"{
      "streams": [
        {"codec_type":"video","codec_name":"h264","width":1080,"height":1920,
         "avg_frame_rate":"30000/1001","side_data_list":[{"rotation":-90}],
         "tags":{"creation_time":"2024-01-02T03:04:05.000000Z"}},
        {"codec_type":"video","codec_name":"mjpeg","width":500,"height":500,
         "disposition":{"attached_pic":1}}
      ],
      "format": {"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"12.5","size":"1000","bit_rate":"640"}
    }"#;
    let p = parse_probe_json(json).unwrap();
    let v = p.video.unwrap();
    assert_eq!(
        v.codec, "h264",
        "cover art must not be picked as the video stream"
    );
    assert_eq!(v.rotation, 270, "-90 normalizes to 270");
    assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
    assert_eq!(
        p.creation_time.as_deref(),
        Some("2024-01-02T03:04:05.000000Z")
    );
    assert_eq!((p.size_bytes, p.bit_rate), (Some(1000), Some(640)));
    assert_eq!(p.duration_secs, Some(12.5));

    // 표지 이미지만 있는 음성 파일 -> 영상 정보 없음
    let json = br#"{"streams":[{"codec_type":"video","codec_name":"mjpeg","width":5,"height":5,
        "disposition":{"attached_pic":1}},{"codec_type":"audio","codec_name":"mp3"}],
        "format":{"format_name":"mp3"}}"#;
    let p = parse_probe_json(json).unwrap();
    assert!(p.video.is_none() && p.audio.is_some());
}

// ---------------------------------------------------------------- T-media-03

#[test]
fn t_media_03_broken_json_and_missing_fields() {
    for bad in [
        &b""[..],
        b"not json",
        b"{\"streams\": [",
        b"{}",
        b"{\"streams\":[],\"format\":{}}",
        b"{\"streams\":[{}]}",
    ] {
        let err = parse_probe_json(bad).unwrap_err();
        assert!(matches!(err, MediaError::InvalidOutput(_)), "{bad:?}");
    }
}

#[test]
fn t_media_03_probe_failure_modes_via_stub() {
    let p = PathBuf::from("x.mp4");
    // 비정상 종료
    let t = stub_tools(|_, _| Ok(fail_output(1, "Invalid data found")));
    match t.probe(&p).unwrap_err() {
        MediaError::ToolFailed { code, stderr, .. } => {
            assert_eq!(code, Some(1));
            assert!(stderr.contains("Invalid data"));
        }
        e => panic!("{e:?}"),
    }
    // 시그널 등으로 코드 없음
    let t = stub_tools(|_, _| Ok(common::fail_output(-1, "")));
    assert!(matches!(
        t.probe(&p).unwrap_err(),
        MediaError::ToolFailed { .. }
    ));
    // 깨진 JSON
    let t = stub_tools(|_, _| Ok(ok_output(b"{broken")));
    assert!(matches!(
        t.probe(&p).unwrap_err(),
        MediaError::InvalidOutput(_)
    ));
    // 타임아웃·출력 초과는 실행기가 돌려준 오류를 그대로 전달
    let t = stub_tools(|_, _| {
        Err(MediaError::Timeout {
            tool: "ffprobe",
            timeout: Duration::from_secs(30),
        })
    });
    assert!(matches!(
        t.probe(&p).unwrap_err(),
        MediaError::Timeout { .. }
    ));
    // 도구 없음 (degraded): 패닉이 아니라 ToolNotFound
    let t = MediaTools::with_paths(None, None, Arc::new(ProcessRunner));
    assert!(!t.has_ffprobe() && !t.has_ffmpeg());
    assert!(matches!(
        t.probe(&p).unwrap_err(),
        MediaError::ToolNotFound { .. }
    ));
}

#[test]
fn t_media_03_probe_passes_path_as_single_argument() {
    use std::sync::Mutex;
    let seen: Arc<Mutex<Vec<OsString>>> = Arc::default();
    let s2 = Arc::clone(&seen);
    let t = stub_tools(move |tool, args| {
        assert_eq!(tool, Tool::Ffprobe);
        *s2.lock().unwrap() = args.to_vec();
        Ok(ok_output(&fixture_json("probe_h264.mp4.json")))
    });
    let path = PathBuf::from("C:/미디어 폴더/-weird name.mp4");
    t.probe(&path).unwrap();
    let args = seen.lock().unwrap();
    assert_eq!(args.last().unwrap(), path.as_os_str());
    assert_eq!(
        args[args.len() - 2],
        OsString::from("-i"),
        "path after -i so a leading '-' is not an option"
    );
}

fn run_opts(timeout_ms: u64, max: usize) -> RunOptions {
    RunOptions {
        timeout: Duration::from_millis(timeout_ms),
        max_stdout: max,
    }
}

#[cfg(windows)]
#[test]
fn t_media_03_real_process_timeout_kills_child() {
    // Arrange: 30초 걸리는 프로세스(ping 30회)
    let args: Vec<OsString> = ["-n", "30", "127.0.0.1"].map(OsString::from).to_vec();
    let started = Instant::now();
    // Act
    let err = ProcessRunner
        .run(
            Tool::Ffprobe,
            std::path::Path::new("ping"),
            &args,
            &run_opts(300, 1 << 20),
        )
        .unwrap_err();
    // Assert: 타임아웃 오류, 제한 시간 근처에서 반환 (회수까지 포함)
    assert!(matches!(err, MediaError::Timeout { .. }), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

#[cfg(windows)]
#[test]
fn t_media_03_real_process_output_limit_and_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.txt");
    fs::write(&big, "x".repeat(2 * 1024 * 1024)).unwrap();
    let args: Vec<OsString> = vec!["/C".into(), "type".into(), big.as_os_str().to_os_string()];
    let err = ProcessRunner
        .run(
            Tool::Ffmpeg,
            std::path::Path::new("cmd"),
            &args,
            &run_opts(20_000, 1000),
        )
        .unwrap_err();
    assert!(
        matches!(err, MediaError::OutputTooLarge { limit: 1000, .. }),
        "{err:?}"
    );

    // 비정상 종료 코드는 Ok(code) 로 전달
    let args: Vec<OsString> = vec!["/C".into(), "exit".into(), "3".into()];
    let out = ProcessRunner
        .run(
            Tool::Ffmpeg,
            std::path::Path::new("cmd"),
            &args,
            &run_opts(20_000, 1000),
        )
        .unwrap();
    assert_eq!(out.code, Some(3));
}

#[test]
fn t_media_03_spawn_failure_is_io_error() {
    let err = ProcessRunner
        .run(
            Tool::Ffprobe,
            std::path::Path::new("Z:/no/such/ffprobe-binary"),
            &[],
            &run_opts(1000, 1000),
        )
        .unwrap_err();
    assert!(matches!(err, MediaError::Io { .. }));
}

#[test]
fn t_media_03_tool_resolution_order() {
    let tmp = tempfile::tempdir().unwrap();
    let mk = |sub: &str| {
        let d = tmp.path().join(sub);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(exe("ffprobe")), b"stub").unwrap();
        d
    };
    let (conf, side, pathd) = (mk("conf"), mk("side"), mk("pathdir"));
    let empty = tmp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let path_env = std::env::join_paths([&pathd]).unwrap();

    // 1) 명시 설정(폴더) 이 최우선
    let cfg = ToolConfig {
        ffprobe: Some(conf.clone()),
        sidecar_dir: Some(side.clone()),
        path_env: Some(path_env.clone()),
        ..Default::default()
    };
    assert_eq!(
        resolve_tool(Tool::Ffprobe, &cfg).unwrap(),
        conf.join(exe("ffprobe"))
    );
    // 명시 설정(파일 직접 지정)
    let cfg_file = ToolConfig {
        ffprobe: Some(conf.join(exe("ffprobe"))),
        ..Default::default()
    };
    assert_eq!(
        resolve_tool(Tool::Ffprobe, &cfg_file).unwrap(),
        conf.join(exe("ffprobe"))
    );
    // 2) 설정 없으면 사이드카 -> 3) PATH
    let cfg = ToolConfig {
        sidecar_dir: Some(side.clone()),
        path_env: Some(path_env.clone()),
        ..Default::default()
    };
    assert_eq!(
        resolve_tool(Tool::Ffprobe, &cfg).unwrap(),
        side.join(exe("ffprobe"))
    );
    let cfg = ToolConfig {
        sidecar_dir: Some(empty.clone()),
        path_env: Some(path_env),
        ..Default::default()
    };
    assert_eq!(
        resolve_tool(Tool::Ffprobe, &cfg).unwrap(),
        pathd.join(exe("ffprobe"))
    );

    // 4) 못 찾으면 ToolNotFound (ffmpeg 는 어디에도 없음)
    let cfg = ToolConfig {
        sidecar_dir: Some(empty.clone()),
        path_env: Some(OsString::from(empty.as_os_str())),
        ..Default::default()
    };
    assert!(matches!(
        resolve_tool(Tool::Ffmpeg, &cfg).unwrap_err(),
        MediaError::ToolNotFound { tool: "ffmpeg", .. }
    ));
    // 5) 잘못된 명시 설정은 PATH 로 넘어가지 않고 오류
    let cfg = ToolConfig {
        ffprobe: Some(empty.join("nope")),
        sidecar_dir: Some(side),
        ..Default::default()
    };
    assert!(matches!(
        resolve_tool(Tool::Ffprobe, &cfg).unwrap_err(),
        MediaError::ToolNotFound { .. }
    ));
}

// ---------------------------------------------------------------- T-media-04

#[test]
fn t_media_04_zero_byte_and_truncated_files() {
    let dir = tempfile::tempdir().unwrap();
    // 형식 판별: 0바이트도 확장자를 따라 판정 (패닉 없음)
    let zero = dir.path().join("empty.mp4");
    fs::write(&zero, b"").unwrap();
    let d = detect(&zero).unwrap();
    assert_eq!(d.kind, Some(MediaKind::Video));

    let Some(tools) = real_tools() else {
        skip(
            "t_media_04_zero_byte_and_truncated_files",
            "ffprobe/ffmpeg not found on PATH",
        );
        return;
    };
    assert!(tools.probe(&zero).is_err());
    let trunc = dir.path().join("trunc.mp4");
    let bytes = fs::read(fixtures_dir().join("tiny.mp4")).unwrap();
    fs::write(&trunc, &bytes[..bytes.len() / 3]).unwrap();
    assert!(
        tools.probe(&trunc).is_err(),
        "moov at end is lost when truncated"
    );
}

#[test]
fn t_media_04_korean_space_and_long_paths() {
    let Some(tools) = real_tools() else {
        skip(
            "t_media_04_korean_space_and_long_paths",
            "ffprobe/ffmpeg not found on PATH",
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let src = fixtures_dir().join("tiny.mp4");

    let kdir = dir.path().join("한글 폴더 (복사본)");
    fs::create_dir_all(&kdir).unwrap();
    let k = kdir.join("영상 파일 #1 [최종].mp4");
    fs::copy(&src, &k).unwrap();
    let p = tools.probe(&k).expect("korean + space path");
    assert_eq!(p.video.unwrap().codec, "h264");

    // 전체 경로 260자 초과 (Windows MAX_PATH)
    let mut long = dir.path().to_path_buf();
    while long.as_os_str().len() < 280 {
        long.push("long_directory_name_segment_0123456789");
    }
    fs::create_dir_all(&long).unwrap();
    let l = long.join("clip.mp4");
    fs::copy(&src, &l).unwrap();
    assert!(l.as_os_str().len() > 260);
    let r = tools.probe(&l);
    // 결과는 ffprobe 빌드에 따라 달라질 수 있어 기록한다 (패닉 없이 Ok/Err 모두 허용하되 오류면 ToolFailed).
    match &r {
        Ok(p) => eprintln!(
            "long path (>260): probe OK, codec {}",
            p.video.as_ref().unwrap().codec
        ),
        Err(e) => {
            eprintln!("long path (>260): probe FAILED: {e}");
            assert!(matches!(e, MediaError::ToolFailed { .. }));
        }
    }
}
