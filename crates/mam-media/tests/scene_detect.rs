//! 장면전환 검출 (`detect_scene_cuts`) — 인자·실패 경로는 스텁 실행기, 실제 검출은 합성 클립 + 실제 FFmpeg.
//! 분류: Validation (T-scene-21) / External (T-scene-22) / Integration + 원본 보호 (T-scene-23) / Concurrency (취소)

use std::collections::hash_map::DefaultHasher;
use std::ffi::OsString;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant, SystemTime};

use mam_media::tool::{
    CancellableProcessRunner, CommandOutput, CommandRunner, MediaTools, ProcessRunner, RunOptions,
    Tool, ToolConfig,
};
use mam_media::{
    MediaError, RawCut, SceneDetectOptions, THRESHOLD_HIGH, THRESHOLD_LOW, THRESHOLD_MEDIUM,
    detect_scene_cuts, scene_timeout,
};

type Reply = Box<dyn Fn() -> Result<CommandOutput, MediaError> + Send + Sync>;

/// 호출(인자·제한)을 기록하고 정해진 결과를 돌려주는 스텁. `run_cancellable` 은 재정의하지 않는다(기본 구현 검증).
struct Recorder {
    calls: AtomicUsize,
    args: Mutex<Vec<Vec<OsString>>>,
    limits: Mutex<Vec<(Duration, usize)>>,
    reply: Reply,
}

impl Recorder {
    fn new(reply: Reply) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            args: Mutex::new(Vec::new()),
            limits: Mutex::new(Vec::new()),
            reply,
        })
    }
}

struct Shared(Arc<Recorder>);

impl CommandRunner for Shared {
    fn run(
        &self,
        _tool: Tool,
        _program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput, MediaError> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        self.0.args.lock().unwrap().push(args.to_vec());
        self.0
            .limits
            .lock()
            .unwrap()
            .push((opts.timeout, opts.max_stdout));
        (self.0.reply)()
    }
}

fn tools_with(rec: &Arc<Recorder>) -> MediaTools {
    MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(Shared(Arc::clone(rec))),
    )
}

fn output(code: i32, stdout: &str, stderr: &str) -> Result<CommandOutput, MediaError> {
    Ok(CommandOutput {
        code: Some(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.to_string(),
    })
}

fn opts() -> SceneDetectOptions {
    SceneDetectOptions::new(THRESHOLD_MEDIUM, Some(60_000))
}

fn never() -> bool {
    false
}

// ── T-scene-21 인자 ─────────────────────────────────────────

#[test]
fn t_scene_21_arguments_follow_the_confirmed_command() {
    // Arrange
    let rec = Recorder::new(Box::new(|| output(0, "", "")));
    let mut tools = tools_with(&rec);
    tools.threads = 3;
    // Act
    let cuts = detect_scene_cuts(&tools, Path::new("D:/v/a.mp4"), &opts(), &never).unwrap();
    // Assert
    assert!(cuts.is_empty());
    let args: Vec<String> = rec.args.lock().unwrap()[0]
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let has = |s: &str| args.iter().any(|a| a == s);
    for flag in [
        "-nostdin",
        "-hide_banner",
        "-noautorotate",
        "-an",
        "-sn",
        "-dn",
    ] {
        assert!(has(flag), "missing {flag}: {args:?}");
    }
    let pair = |k: &str, v: &str| args.windows(2).any(|w| w[0] == k && w[1] == v);
    assert!(pair("-v", "error"));
    assert!(pair("-threads", "3"));
    assert!(pair("-map", "[k]"));
    assert!(pair("-f", "null"));
    assert_eq!(args.last().map(String::as_str), Some("-"), "{args:?}");
    assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1);
    assert!(
        !has("-y") && !has("-n"),
        "must never write a file: {args:?}"
    );
    let i = args.iter().position(|a| a == "-i").unwrap();
    assert!(args[i + 1].replace('\\', "/").ends_with("v/a.mp4"));
    assert!(!has("-vf"), "v2 uses -filter_complex: {args:?}");
    let g = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
    // 영상 스트림 선택은 그래프 입력 라벨이다 (커버 아트 제외 `V`)
    assert!(g.starts_with("[0:V:0]scale=w=320:h=-2"), "{g}");
    // 한 번의 디코딩에서 일반 + normalize 두 체인, 임계값은 두 체인 모두, 비버퍼 출력
    assert!(g.contains("split=3"), "{g}");
    assert_eq!(g.matches("scdet=threshold=10,").count(), 2, "{g}");
    assert_eq!(
        g.matches("metadata=mode=select:key=lavfi.scd.time").count(),
        2,
        "{g}"
    );
    assert_eq!(
        g.matches("metadata=mode=print:file=-:direct=1").count(),
        2,
        "{g}"
    );
    assert!(
        g.contains("normalize=smoothing=0:independence=0,format=gray"),
        "{g}"
    );
    assert!(g.ends_with("[p]nullsink;[n]nullsink"), "{g}");
}

#[test]
fn t_scene_21_threshold_constants_and_range_validation() {
    assert_eq!(
        (THRESHOLD_LOW, THRESHOLD_MEDIUM, THRESHOLD_HIGH),
        (20.0, 10.0, 5.0)
    );
    // 범위 밖·비유한 임계값은 FFmpeg 를 띄우기 전에 거부한다
    let rec = Recorder::new(Box::new(|| output(0, "", "")));
    let tools = tools_with(&rec);
    for bad in [0.0, -1.0, 100.5, f32::NAN, f32::INFINITY] {
        let err = detect_scene_cuts(
            &tools,
            Path::new("a.mp4"),
            &SceneDetectOptions::new(bad, None),
            &never,
        )
        .unwrap_err();
        assert!(
            matches!(err, MediaError::InvalidOutput(_)),
            "{bad}: {err:?}"
        );
    }
    assert_eq!(rec.calls.load(Ordering::SeqCst), 0);
}

// ── T-scene-22 외부 실패 ────────────────────────────────────

#[test]
fn t_scene_22_limits_follow_duration_and_output_cap() {
    // Arrange
    let rec = Recorder::new(Box::new(|| output(0, "", "")));
    let tools = tools_with(&rec);
    // Act
    detect_scene_cuts(
        &tools,
        Path::new("a.mp4"),
        &SceneDetectOptions::new(10.0, Some(3_600_000)),
        &never,
    )
    .unwrap();
    detect_scene_cuts(
        &tools,
        Path::new("a.mp4"),
        &SceneDetectOptions::new(10.0, None),
        &never,
    )
    .unwrap();
    // Assert - 전역 30초 타임아웃이 아니라 호출별 제한
    let limits = rec.limits.lock().unwrap().clone();
    assert_eq!(
        limits[0],
        (scene_timeout(Some(3_600_000)), 16 * 1024 * 1024)
    );
    assert_eq!(limits[1], (Duration::from_secs(7200), 16 * 1024 * 1024));
}

#[test]
fn t_scene_22_runner_errors_pass_through() {
    for (make, label) in [
        (
            (|| {
                Err(MediaError::Timeout {
                    tool: "ffmpeg",
                    timeout: Duration::from_secs(1),
                })
            }) as fn() -> Result<CommandOutput, MediaError>,
            "timeout",
        ),
        (
            || {
                Err(MediaError::OutputTooLarge {
                    tool: "ffmpeg",
                    limit: 1,
                })
            },
            "output",
        ),
    ] {
        let rec = Recorder::new(Box::new(make));
        let err =
            detect_scene_cuts(&tools_with(&rec), Path::new("a.mp4"), &opts(), &never).unwrap_err();
        match label {
            "timeout" => assert!(matches!(err, MediaError::Timeout { .. }), "{err:?}"),
            _ => assert!(matches!(err, MediaError::OutputTooLarge { .. }), "{err:?}"),
        }
    }
}

#[test]
fn t_scene_22_nonzero_exit_is_tool_failed_and_no_video_is_unsupported() {
    // Arrange / Act / Assert - 일반 실패
    let rec = Recorder::new(Box::new(|| {
        output(183, "", "Invalid data found when processing input")
    }));
    let err =
        detect_scene_cuts(&tools_with(&rec), Path::new("a.mp4"), &opts(), &never).unwrap_err();
    assert!(
        matches!(
            err,
            MediaError::ToolFailed {
                code: Some(183),
                ..
            }
        ),
        "{err:?}"
    );
    // 영상 스트림 없음 (PoC 로 확인한 stderr 문구)
    let rec = Recorder::new(Box::new(|| {
        output(
            127,
            "",
            "Stream map '' matches no streams.\nTo ignore this, add a trailing '?' to the map.",
        )
    }));
    let err =
        detect_scene_cuts(&tools_with(&rec), Path::new("a.wav"), &opts(), &never).unwrap_err();
    assert!(matches!(err, MediaError::Unsupported(_)), "{err:?}");
}

#[test]
fn t_scene_22_missing_ffmpeg_is_tool_not_found() {
    let rec = Recorder::new(Box::new(|| output(0, "", "")));
    let tools = MediaTools::with_paths(None, None, Arc::new(Shared(Arc::clone(&rec))));
    let err = detect_scene_cuts(&tools, Path::new("a.mp4"), &opts(), &never).unwrap_err();
    assert!(matches!(err, MediaError::ToolNotFound { .. }), "{err:?}");
    assert_eq!(rec.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn t_scene_22_precancelled_never_starts_the_process() {
    // Arrange - 기본 `run_cancellable` (스텁은 재정의하지 않음)
    let rec = Recorder::new(Box::new(|| output(0, "", "")));
    // Act
    let err =
        detect_scene_cuts(&tools_with(&rec), Path::new("a.mp4"), &opts(), &|| true).unwrap_err();
    // Assert
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    assert_eq!(rec.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn t_scene_22_parses_stdout_of_a_successful_run() {
    let rec = Recorder::new(Box::new(|| {
        output(
            0,
            "frame:0 pts:1 pts_time:1\r\nlavfi.scd.mafd=60\r\nlavfi.scd.score=60\r\nlavfi.scd.time=1.04\r\n",
            "",
        )
    }));
    let cuts = detect_scene_cuts(&tools_with(&rec), Path::new("a.mp4"), &opts(), &never).unwrap();
    assert_eq!(
        cuts,
        vec![RawCut {
            ms: 1040,
            score: 60.0
        }]
    );
}

#[cfg(windows)]
fn ping_args() -> Vec<OsString> {
    ["-n", "30", "127.0.0.1"].map(OsString::from).to_vec()
}

#[cfg(windows)]
fn run_opts() -> RunOptions {
    RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 1 << 20,
    }
}

#[cfg(windows)]
#[test]
fn t_scene_22_process_runner_kills_the_child_when_cancel_turns_true() {
    // Arrange - 30초 걸리는 프로세스, 300ms 뒤 취소 클로저가 true
    let started = Instant::now();
    let cancel = move || started.elapsed() > Duration::from_millis(300);
    // Act
    let err = ProcessRunner
        .run_cancellable(
            Tool::Ffmpeg,
            Path::new("ping"),
            &ping_args(),
            &run_opts(),
            &cancel,
        )
        .unwrap_err();
    // Assert - 프로세스 종료나 60초 제한을 기다리지 않는다
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(windows)]
#[test]
fn t_scene_22_cancellable_runner_honours_either_shutdown_flag_or_closure() {
    // Arrange
    let flag = Arc::new(AtomicBool::new(false));
    let runner = CancellableProcessRunner::new(Arc::clone(&flag));
    let setter = {
        let flag = Arc::clone(&flag);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
        })
    };
    let started = Instant::now();
    // Act - 클로저는 계속 false, 종료 플래그만 켜진다
    let err = runner
        .run_cancellable(
            Tool::Ffmpeg,
            Path::new("ping"),
            &ping_args(),
            &run_opts(),
            &never,
        )
        .unwrap_err();
    setter.join().unwrap();
    // Assert
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    // 종료 플래그가 이미 켜져 있으면 클로저와 무관하게 띄우지 않는다
    let err = runner
        .run_cancellable(
            Tool::Ffmpeg,
            Path::new("Z:/no/such/binary"),
            &[],
            &run_opts(),
            &never,
        )
        .unwrap_err();
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
}

// ── T-scene-23 실제 FFmpeg + 원본 보호 ──────────────────────

/// 번들 FFmpeg(`src-tauri/binaries`, DLL 폴더를 PATH 앞에 둔다) 우선, 없으면 PATH. 둘 다 없으면 `None` (테스트 SKIP).
fn real_tools() -> Option<(MediaTools, PathBuf)> {
    static PATH_ONCE: Once = Once::new();
    let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/binaries");
    let bundled = bin.join("ffmpeg-x86_64-pc-windows-msvc.exe");
    if bundled.is_file() {
        PATH_ONCE.call_once(|| {
            let mut paths = vec![bin.join("dlls")];
            paths.extend(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ));
            if let Ok(joined) = std::env::join_paths(paths) {
                // 같은 프로세스의 다른 테스트가 PATH 를 쓰기 전에 한 번만 설정한다 (이 파일의 테스트만 PATH 를 바꾼다)
                unsafe { std::env::set_var("PATH", joined) };
            }
        });
        let cfg = ToolConfig {
            ffmpeg: Some(bundled.clone()),
            ..ToolConfig::default()
        };
        let tools = MediaTools::discover(&cfg, Arc::new(ProcessRunner));
        return tools.has_ffmpeg().then_some((tools, bundled));
    }
    let tools = MediaTools::discover(&ToolConfig::default(), Arc::new(ProcessRunner));
    tools.has_ffmpeg().then(|| (tools, PathBuf::from("ffmpeg")))
}

/// 검은색 1.0s / 흰색 1.2s / 회색 1.6s / 검은색 1.0s — 25fps, 컷 = 1000 / 2200 / 3800 ms (프레임 40ms).
fn make_clip(ffmpeg: &Path, out: &Path) -> bool {
    let graph = "color=black:s=320x240:r=25:d=1[a];color=white:s=320x240:r=25:d=1.2[b];\
                 color=gray:s=320x240:r=25:d=1.6[c];color=black:s=320x240:r=25:d=1[d];\
                 [a][b][c][d]concat=n=4:v=1:a=0[v]";
    std::process::Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-filter_complex",
        ])
        .arg(graph)
        .args([
            "-map", "[v]", "-c:v", "mpeg4", "-q:v", "3", "-pix_fmt", "yuv420p",
        ])
        .arg(out)
        .status()
        .is_ok_and(|s| s.success())
        && out.is_file()
}

fn fingerprint(path: &Path) -> (u64, SystemTime, u64) {
    let bytes = fs::read(path).unwrap();
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    let meta = fs::metadata(path).unwrap();
    (h.finish(), meta.modified().unwrap(), meta.len())
}

fn listing(dir: &Path) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                e.metadata().unwrap().len(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn t_scene_23_detects_hard_cuts_within_one_frame_and_leaves_the_source_untouched() {
    let Some((tools, ffmpeg)) = real_tools() else {
        eprintln!("SKIP t_scene_23: ffmpeg not available");
        return;
    };
    // Arrange - 임시 폴더의 합성 클립 (사용자 폴더 접근 없음)
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mkv");
    if !make_clip(&ffmpeg, &clip) {
        eprintln!("SKIP t_scene_23: cannot synthesize the clip with this ffmpeg build");
        return;
    }
    let (before, listing_before) = (fingerprint(&clip), listing(dir.path()));
    // Act
    let cuts = detect_scene_cuts(
        &tools,
        &clip,
        &SceneDetectOptions::new(THRESHOLD_MEDIUM, Some(4_800)),
        &never,
    )
    .unwrap();
    // Assert - 검출 (±1프레임 = 40ms)
    let got: Vec<i64> = cuts.iter().map(|c| c.ms).collect();
    assert_eq!(got.len(), 3, "{got:?}");
    for (g, want) in got.iter().zip([1000i64, 2200, 3800]) {
        assert!((g - want).abs() <= 40, "{got:?}");
    }
    assert!(cuts.iter().all(|c| c.score > 10.0), "{cuts:?}");
    // Assert - 원본 보호: 내용 해시·수정 시각·크기·폴더 목록 불변
    assert_eq!(fingerprint(&clip), before);
    assert_eq!(listing(dir.path()), listing_before);
}

/// 밝은 하드컷(1.0s) + 어두운 하드컷(3.0s). 어두운 구간은 휘도 16..41 로 눌러(평균 휘도 약 25~28) 서로 다른 내용 둘을 잇는다.
/// 밝은 구간 `testsrc2`→`smptebars` (1.0s), 밝→어둠 (2.0s), 어두운 `testsrc2`→어두운 `smptebars` (3.0s).
fn make_dark_clip(ffmpeg: &Path, out: &Path) -> bool {
    let dark = "lutyuv=y=16+val/10:u=128:v=128";
    let graph = format!(
        "testsrc2=s=320x240:r=25:d=1,format=yuv420p[a];smptebars=s=320x240:r=25:d=1,format=yuv420p[b];         testsrc2=s=320x240:r=25:d=1,format=yuv420p,{dark}[c];         smptebars=s=320x240:r=25:d=1,format=yuv420p,{dark}[d];         [a][b][c][d]concat=n=4:v=1:a=0[v]"
    );
    std::process::Command::new(ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-filter_complex",
        ])
        .arg(graph)
        .args([
            "-map", "[v]", "-c:v", "mpeg4", "-q:v", "3", "-pix_fmt", "yuv420p",
        ])
        .arg(out)
        .status()
        .is_ok_and(|s| s.success())
        && out.is_file()
}

/// v1 (일반 체인만) 을 그대로 재현해 대조군으로 쓴다 — 어두운 컷이 일반 체인에서는 임계값 아래임을 증거로 남긴다.
fn plain_chain_cuts(ffmpeg: &Path, clip: &Path, threshold: f32) -> Vec<RawCut> {
    let vf = format!(
        "scale=w=320:h=-2:flags=fast_bilinear,format=gray,scdet=threshold={threshold},         metadata=mode=select:key=lavfi.scd.time,metadata=mode=print:file=-"
    );
    let out = std::process::Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-v", "error", "-i"])
        .arg(clip)
        .args(["-an", "-sn", "-dn", "-map", "0:V:0", "-vf"])
        .arg(vf)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(out.status.success());
    mam_media::parse_scdet_output(&String::from_utf8_lossy(&out.stdout))
}

#[test]
fn t_scene_25_v2_finds_a_dark_hard_cut_that_the_plain_chain_misses() {
    let Some((tools, ffmpeg)) = real_tools() else {
        eprintln!("SKIP t_scene_25: ffmpeg not available");
        return;
    };
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("dark.mkv");
    if !make_dark_clip(&ffmpeg, &clip) {
        eprintln!("SKIP t_scene_25: cannot synthesize the clip with this ffmpeg build");
        return;
    }
    let (before, listing_before) = (fingerprint(&clip), listing(dir.path()));
    let near = |cuts: &[RawCut], want: i64| cuts.iter().any(|c| (c.ms - want).abs() <= 40);
    // Act
    let cuts = detect_scene_cuts(
        &tools,
        &clip,
        &SceneDetectOptions::new(THRESHOLD_MEDIUM, Some(4_000)),
        &never,
    )
    .unwrap();
    let plain = plain_chain_cuts(&ffmpeg, &clip, THRESHOLD_MEDIUM);
    // Assert - v2 합집합: 밝은 컷(1.0s)·밝→어둠(2.0s)·어두운 컷(3.0s) 모두, 같은 ms 중복 없음
    for want in [1000, 2000, 3000] {
        assert!(near(&cuts, want), "v2 missing {want}: {cuts:?}");
    }
    assert_eq!(cuts.len(), 3, "{cuts:?}");
    assert!(cuts.windows(2).all(|w| w[0].ms < w[1].ms), "{cuts:?}");
    // Assert - 대조군: 일반 체인은 어두운 컷을 놓친다 (v1 한계), 밝은 컷은 찾는다
    assert!(near(&plain, 1000) && near(&plain, 2000), "{plain:?}");
    assert!(
        !near(&plain, 3000),
        "plain chain unexpectedly saw the dark cut: {plain:?}"
    );
    // Assert - 원본 보호
    assert_eq!(fingerprint(&clip), before);
    assert_eq!(listing(dir.path()), listing_before);
}

#[test]
fn t_scene_25_clip_without_cuts_still_succeeds_with_the_two_chain_graph() {
    let Some((tools, ffmpeg)) = real_tools() else {
        eprintln!("SKIP t_scene_25: ffmpeg not available");
        return;
    };
    // Arrange - 컷이 한쪽 체인에만 있거나 전혀 없는 입력에서 출력 스트림이 비어 실패하면 안 된다
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("still.mkv");
    let ok = std::process::Command::new(&ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("color=gray:s=320x240:r=25:d=2")
        .args(["-c:v", "mpeg4"])
        .arg(&clip)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIP t_scene_25: cannot synthesize the clip");
        return;
    }
    // Act / Assert
    let cuts = detect_scene_cuts(&tools, &clip, &opts(), &never).unwrap();
    assert!(cuts.is_empty(), "{cuts:?}");
}

#[test]
fn t_scene_23_audio_only_file_is_unsupported_not_a_crash() {
    let Some((tools, ffmpeg)) = real_tools() else {
        eprintln!("SKIP t_scene_23: ffmpeg not available");
        return;
    };
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    let ok = std::process::Command::new(&ffmpeg)
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg("sine=d=1")
        .arg(&wav)
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIP t_scene_23: cannot synthesize audio");
        return;
    }
    // Act
    let err = detect_scene_cuts(&tools, &wav, &opts(), &never).unwrap_err();
    // Assert
    assert!(matches!(err, MediaError::Unsupported(_)), "{err:?}");
}

#[test]
fn t_scene_23_missing_input_is_an_io_free_tool_failure() {
    let Some((tools, _)) = real_tools() else {
        eprintln!("SKIP t_scene_23: ffmpeg not available");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let err = detect_scene_cuts(&tools, &dir.path().join("nope.mkv"), &opts(), &never).unwrap_err();
    assert!(matches!(err, MediaError::ToolFailed { .. }), "{err:?}");
}
