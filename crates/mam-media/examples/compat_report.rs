//! P1-5 포맷 호환표 측정 — 폴더의 모든 파일에 대해 메타데이터 / 썸네일 / WebView2 재생 판정을 실행해
//! 마크다운 표로 출력한다.
//!
//! ```text
//! cargo run -p mam-media --example compat_report -- <샘플 폴더> <캐시 폴더>
//! ```
//!
//! 샘플은 `gen_samples` 로 만든다. 캐시 폴더는 MAM 데이터 디렉터리를 흉내 낸 임시 폴더로 지정할 것.
//! FFmpeg 경로는 환경변수 `MAM_FFMPEG_DIR`(폴더 또는 파일, 없으면 PATH) 로 지정할 수 있다.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use mam_media::format::{MediaKind, detect};
use mam_media::thumbnail::{ThumbnailOptions, create_thumbnails};
use mam_media::{
    MediaTools, Playback, ProcessRunner, ToolConfig, image_in_webview, playback_of, read_image_meta,
};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn short(e: &impl std::fmt::Display) -> String {
    let s = e.to_string().replace(['\r', '\n', '|'], " ");
    let s: String = s.chars().take(110).collect();
    s
}

fn verdict(p: Playback) -> &'static str {
    match p {
        Playback::Playable => "가능",
        Playback::Conditional => "조건부",
        Playback::NotPlayable => "불가",
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("usage: compat_report <samples> <cache>"));
    let cache = PathBuf::from(args.next().expect("usage: compat_report <samples> <cache>"));
    let dir = std::env::var_os("MAM_FFMPEG_DIR").map(PathBuf::from);
    let cfg = ToolConfig {
        ffprobe: dir.clone(),
        ffmpeg: dir,
        ..Default::default()
    };
    let tools = MediaTools::discover(&cfg, Arc::new(ProcessRunner));
    println!(
        "ffprobe={} ffmpeg={}\n",
        tools.has_ffprobe(),
        tools.has_ffmpeg()
    );
    println!("| 파일 | 종류 | 메타데이터 | 썸네일 | 재생/표시(WebView2) |");
    println!("|---|---|---|---|---|");

    let mut files = Vec::new();
    walk(&root, &mut files);
    for (i, path) in files.iter().enumerate() {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let det = match detect(path) {
            Ok(d) => d,
            Err(e) => {
                println!("| {name} | ? | 판별 실패: {} | - | - |", short(&e));
                continue;
            }
        };
        let kind = match det.kind {
            Some(MediaKind::Image) => "이미지",
            Some(MediaKind::Video) => "영상",
            Some(MediaKind::Audio) => "음성",
            None => "비대상",
        };
        let mismatch = if det.mismatch {
            " (확장자 불일치)"
        } else {
            ""
        };

        let (meta, play) = match det.kind {
            Some(MediaKind::Image) => {
                let m = match read_image_meta(path) {
                    Ok(m) => match (m.width, m.height) {
                        (Some(w), Some(h)) => format!(
                            "OK {w}x{h} 방향{} {}",
                            m.orientation,
                            m.taken_at.unwrap_or_default()
                        ),
                        _ => {
                            // image crate 가 크기를 모르는 형식은 ffprobe 로 보충
                            match tools.probe(path) {
                                Ok(p) => match p.video {
                                    Some(v) => {
                                        format!("OK(ffprobe) {}x{} {}", v.width, v.height, v.codec)
                                    }
                                    None => "부분: 크기 없음".into(),
                                },
                                Err(e) => format!("부분: 크기 없음 (ffprobe: {})", short(&e)),
                            }
                        }
                    },
                    Err(e) => format!("실패: {}", short(&e)),
                };
                (m, verdict(image_in_webview(&det.extension)).to_string())
            }
            Some(MediaKind::Video | MediaKind::Audio) => match tools.probe(path) {
                Ok(p) => {
                    let v = playback_of(&p);
                    let m = format!(
                        "OK {} {:.1}s {} {} {}",
                        p.container.split(',').next().unwrap_or(""),
                        p.duration_secs.unwrap_or(0.0),
                        p.video
                            .as_ref()
                            .map(|v| format!("{}x{} {}", v.width, v.height, v.codec))
                            .unwrap_or_default(),
                        p.audio
                            .as_ref()
                            .map(|a| a.codec.clone())
                            .unwrap_or_default(),
                        p.creation_time.unwrap_or_default()
                    );
                    (m, format!("{} ({})", verdict(v.playback), v.reason))
                }
                Err(e) => (format!("실패: {}", short(&e)), "-".into()),
            },
            None => ("-".into(), "-".into()),
        };

        let thumb = if det.kind.is_none() {
            "-".to_string()
        } else {
            let t = Instant::now();
            match create_thumbnails(
                &tools,
                path,
                i as i64 + 1,
                &cache,
                &ThumbnailOptions::default(),
            ) {
                Ok(o) => format!(
                    "OK {:?} {}x{} {}ms",
                    o.method,
                    o.large_size.0,
                    o.large_size.1,
                    t.elapsed().as_millis()
                ),
                Err(mam_media::MediaError::Unsupported(why)) => format!("해당 없음 ({why})"),
                Err(e) => format!("실패: {}", short(&e)),
            }
        };
        println!("| {name} | {kind}{mismatch} | {meta} | {thumb} | {play} |");
    }
}
