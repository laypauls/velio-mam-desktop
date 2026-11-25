//! T-ai-09~11, 15 - 로컬 STT (whisper-cli 사이드카). 분류: Happy / External / Business rule / Validation.
//! 외부 프로세스는 `CommandRunner` 스텁으로 대체한다. 실제 whisper/ffmpeg/모델이 있으면 e2e 1건을 추가로 실행한다.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mam_ai::AiError;
use mam_ai::local::stt::{
    CancelToken, CommandOutput, CommandRunner, Language, ProcessRunner, RunOptions, SttConfig,
    SttEngine, SttPaths, Tool, parse_whisper_json, resolve_tool,
};

/// 실제 whisper-cli 0.x 출력과 같은 구조 (불필요한 필드는 줄였다).
const SAMPLE_JSON: &str = r#"{
 "systeminfo": "WHISPER : CPU",
 "model": {"type": "base", "multilingual": true},
 "params": {"language": "auto", "translate": false},
 "result": {"language": "ko"},
 "transcription": [
  {"timestamps": {"from": "00:00:00,000", "to": "00:00:02,800"}, "offsets": {"from": 0, "to": 2800}, "text": " 오늘 회의에서는"},
  {"timestamps": {"from": "00:00:02,800", "to": "00:00:05,300"}, "offsets": {"from": 2800, "to": 5300}, "text": " 일정을 논의했습니다."},
  {"timestamps": {"from": "00:00:05,300", "to": "00:00:06,000"}, "offsets": {"from": 5300, "to": 6000}, "text": "   "}
 ]
}"#;

// ---------- T-ai-09 ----------

#[test]
fn t_ai_09_parses_segments_and_detected_language() {
    let t = parse_whisper_json(SAMPLE_JSON).unwrap();

    assert_eq!(t.language, "ko");
    assert_eq!(t.segments.len(), 2, "blank segment is dropped");
    assert_eq!((t.segments[0].start_ms, t.segments[0].end_ms), (0, 2800));
    assert_eq!(t.segments[0].text, "오늘 회의에서는");
    assert_eq!(t.segments[1].start_ms, 2800);
    assert_eq!(t.full_text(), "오늘 회의에서는 일정을 논의했습니다.");
}

#[test]
fn t_ai_09_rejects_malformed_output() {
    // BOM 은 허용
    assert!(parse_whisper_json(&format!("\u{feff}{SAMPLE_JSON}")).is_ok());
    // 깨진 JSON / 음수 시간 / 끝 < 시작
    for bad in [
        "{not json",
        "",
        r#"{"transcription":[{"offsets":{"from":-5,"to":10},"text":"x"}]}"#,
        r#"{"transcription":[{"offsets":{"from":100,"to":10},"text":"x"}]}"#,
        r#"{"transcription":[{"text":"no offsets"}]}"#,
    ] {
        assert!(
            matches!(parse_whisper_json(bad), Err(AiError::ToolOutput { .. })),
            "should reject {bad:?}"
        );
    }
    // 세그먼트 없음(무음)은 빈 전사로 정상
    let t = parse_whisper_json(r#"{"result":{"language":"en"},"transcription":[]}"#).unwrap();
    assert!(t.segments.is_empty() && t.language == "en");
}

// ---------- 스텁 실행기 ----------

type Handler = dyn Fn(Tool, &[OsString]) -> Result<CommandOutput, AiError> + Send + Sync;

struct Stub {
    handler: Box<Handler>,
    calls: Mutex<Vec<(Tool, Vec<OsString>)>>,
}

impl CommandRunner for Stub {
    fn run(
        &self,
        tool: Tool,
        _p: &Path,
        args: &[OsString],
        _o: &RunOptions,
    ) -> Result<CommandOutput, AiError> {
        self.calls.lock().unwrap().push((tool, args.to_vec()));
        (self.handler)(tool, args)
    }
}

fn ok(stdout: &str) -> Result<CommandOutput, AiError> {
    Ok(CommandOutput {
        code: Some(0),
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn arg_after(args: &[OsString], flag: &str) -> PathBuf {
    let i = args
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("no {flag}"));
    PathBuf::from(&args[i + 1])
}

/// ffmpeg 는 마지막 인자 경로에 10초짜리 WAV 크기 파일을, whisper 는 `-of` 경로 + ".json" 을 쓴다.
fn working_stub() -> Arc<Stub> {
    Arc::new(Stub {
        calls: Mutex::new(Vec::new()),
        handler: Box::new(|tool, args| {
            match tool {
                Tool::Ffmpeg => {
                    let out = PathBuf::from(args.last().unwrap());
                    std::fs::write(out, vec![0u8; 44 + 16_000 * 2 * 10]).unwrap();
                }
                Tool::WhisperCli => {
                    let base = arg_after(args, "-of");
                    std::fs::write(format!("{}.json", base.display()), SAMPLE_JSON).unwrap();
                }
            }
            ok("")
        }),
    })
}

fn engine(runner: Arc<dyn CommandRunner>, cfg: SttConfig) -> SttEngine {
    SttEngine::new(
        runner,
        SttPaths {
            whisper_cli: "whisper-cli.exe".into(),
            ffmpeg: "ffmpeg.exe".into(),
            model: "model.bin".into(),
        },
        cfg,
    )
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

// ---------- T-ai-09/11: 엔진 흐름과 원본 보호 ----------

#[test]
fn t_ai_11_temp_files_only_under_work_dir_and_removed_afterwards() {
    // Arrange
    let media = tempfile::tempdir().unwrap(); // 사용자 미디어 폴더
    let work = tempfile::tempdir().unwrap(); // MAMData/tmp
    let input = media.path().join("interview.mp3");
    std::fs::write(&input, b"original audio bytes").unwrap();
    let before = (
        std::fs::read(&input).unwrap(),
        std::fs::metadata(&input).unwrap().modified().unwrap(),
    );
    let stub = working_stub();
    let eng = engine(
        stub.clone(),
        SttConfig {
            threads: 3,
            language: Language::Fixed("ko".into()),
            ..Default::default()
        },
    );

    // Act
    let t = eng.transcribe(&input, work.path(), None).unwrap();

    // Assert: 전사 결과 + 모든 출력 경로가 work_dir 아래
    assert_eq!(t.language, "ko");
    let calls = stub.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let ffmpeg_out = PathBuf::from(calls[0].1.last().unwrap());
    let whisper_in = arg_after(&calls[1].1, "-f");
    let whisper_out = arg_after(&calls[1].1, "-of");
    for p in [&ffmpeg_out, &whisper_in, &whisper_out] {
        assert!(
            p.starts_with(work.path()),
            "{} escapes work dir",
            p.display()
        );
        assert!(!p.starts_with(media.path()));
    }
    // 원본은 입력(-i)으로만 쓰인다
    assert_eq!(arg_after(&calls[0].1, "-i"), input);
    assert_eq!(arg_after(&calls[1].1, "-l"), PathBuf::from("ko"));
    assert_eq!(arg_after(&calls[1].1, "-t"), PathBuf::from("3"));
    // 종료 후 임시 폴더는 비어 있고, 미디어 폴더와 원본은 그대로
    assert!(
        entries(work.path()).is_empty(),
        "work dir must be clean: {:?}",
        entries(work.path())
    );
    assert_eq!(entries(media.path()), vec![input.clone()]);
    assert_eq!(
        (
            std::fs::read(&input).unwrap(),
            std::fs::metadata(&input).unwrap().modified().unwrap()
        ),
        before
    );
}

#[test]
fn t_ai_11_temp_files_removed_on_failure_and_cancel_too() {
    let media = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = media.path().join("a.wav");
    std::fs::write(&input, b"x").unwrap();

    // whisper 가 비정상 종료
    let stub = Arc::new(Stub {
        calls: Mutex::new(Vec::new()),
        handler: Box::new(|tool, args| match tool {
            Tool::Ffmpeg => {
                std::fs::write(args.last().unwrap(), vec![0u8; 44 + 32_000]).unwrap();
                ok("")
            }
            Tool::WhisperCli => Ok(CommandOutput {
                code: Some(3),
                stdout: String::new(),
                stderr: "boom: bad model".into(),
            }),
        }),
    });
    let err = engine(stub, SttConfig::default())
        .transcribe(&input, work.path(), None)
        .unwrap_err();
    assert!(
        matches!(&err, AiError::ToolFailed { tool: "whisper-cli", code: Some(3), stderr } if stderr.contains("bad model")),
        "{err}"
    );
    assert!(entries(work.path()).is_empty());

    // 취소
    let stub = Arc::new(Stub {
        calls: Mutex::new(Vec::new()),
        handler: Box::new(|_, _| Err(AiError::Cancelled { tool: "ffmpeg" })),
    });
    let err = engine(stub, SttConfig::default())
        .transcribe(&input, work.path(), None)
        .unwrap_err();
    assert!(matches!(err, AiError::Cancelled { .. }));
    assert!(entries(work.path()).is_empty());
    assert_eq!(std::fs::read(&input).unwrap(), b"x");
}

#[test]
fn t_ai_09_timeout_scales_with_audio_length_and_empty_audio_skips_whisper() {
    let media = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = media.path().join("a.wav");
    std::fs::write(&input, b"x").unwrap();

    // 100초 WAV: 제한 시간은 max(floor 10s, 100s x 6) 이어야 한다
    let seen = Arc::new(Mutex::new(Vec::<Duration>::new()));
    struct Spy(Arc<Mutex<Vec<Duration>>>);
    impl CommandRunner for Spy {
        fn run(
            &self,
            tool: Tool,
            _p: &Path,
            args: &[OsString],
            o: &RunOptions,
        ) -> Result<CommandOutput, AiError> {
            self.0.lock().unwrap().push(o.timeout);
            match tool {
                Tool::Ffmpeg => {
                    std::fs::write(args.last().unwrap(), vec![0u8; 44 + 32_000 * 100]).unwrap()
                }
                Tool::WhisperCli => std::fs::write(
                    format!("{}.json", arg_after(args, "-of").display()),
                    SAMPLE_JSON,
                )
                .unwrap(),
            }
            Ok(CommandOutput {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }
    let cfg = SttConfig {
        timeout_floor: Duration::from_secs(10),
        timeout_rtf: 6.0,
        ..Default::default()
    };
    engine(Arc::new(Spy(seen.clone())), cfg)
        .transcribe(&input, work.path(), None)
        .unwrap();
    assert_eq!(seen.lock().unwrap()[1], Duration::from_secs(600));

    // 헤더만 있는 WAV(샘플 0) -> whisper 를 실행하지 않고 빈 전사
    let stub = Arc::new(Stub {
        calls: Mutex::new(Vec::new()),
        handler: Box::new(|_, args| {
            std::fs::write(args.last().unwrap(), vec![0u8; 44]).unwrap();
            ok("")
        }),
    });
    let t = engine(stub.clone(), SttConfig::default())
        .transcribe(&input, work.path(), None)
        .unwrap();
    assert!(t.segments.is_empty());
    assert_eq!(
        stub.calls.lock().unwrap().len(),
        1,
        "whisper must not run on empty audio"
    );
}

#[test]
fn t_ai_15_missing_input_is_io_error_and_creates_nothing() {
    let work = tempfile::tempdir().unwrap();
    let stub = working_stub();
    let err = engine(stub.clone(), SttConfig::default())
        .transcribe(&work.path().join("nope.mp3"), work.path(), None)
        .unwrap_err();
    assert!(matches!(err, AiError::Io { .. }));
    assert!(stub.calls.lock().unwrap().is_empty());
    assert!(entries(work.path()).is_empty());
}

// ---------- T-ai-10: 외부 실패 ----------

#[test]
fn t_ai_10_tool_not_found_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let empty_path = OsString::from(dir.path());
    // PATH/사이드카 어디에도 없다
    let e = resolve_tool(Tool::WhisperCli, None, Some(dir.path()), Some(&empty_path)).unwrap_err();
    assert!(matches!(
        e,
        AiError::ToolNotFound {
            tool: "whisper-cli",
            ..
        }
    ));
    // 명시 설정이 틀리면 PATH 로 넘어가지 않고 실패한다
    let e = resolve_tool(Tool::Ffmpeg, Some(&dir.path().join("x.exe")), None, None).unwrap_err();
    assert!(matches!(e, AiError::ToolNotFound { tool: "ffmpeg", .. }));
    // 사이드카 폴더에서 찾는다
    let exe = dir.path().join(if cfg!(windows) {
        "whisper-cli.exe"
    } else {
        "whisper-cli"
    });
    std::fs::write(&exe, b"").unwrap();
    assert_eq!(
        resolve_tool(Tool::WhisperCli, None, Some(dir.path()), Some(&empty_path)).unwrap(),
        exe
    );
    // 폴더를 명시해도 된다
    assert_eq!(
        resolve_tool(Tool::WhisperCli, Some(dir.path()), None, None).unwrap(),
        exe
    );
}

#[cfg(windows)]
fn sleeper() -> (PathBuf, Vec<OsString>) {
    // `ping -n 30 127.0.0.1` 은 약 30초 걸리는 표준 Windows 프로세스다.
    (
        PathBuf::from("ping"),
        vec!["-n".into(), "30".into(), "127.0.0.1".into()],
    )
}

#[cfg(windows)]
#[test]
fn t_ai_10_process_runner_kills_on_timeout_and_cancel() {
    let (prog, args) = sleeper();
    // 타임아웃
    let started = std::time::Instant::now();
    let err = ProcessRunner
        .run(
            Tool::WhisperCli,
            &prog,
            &args,
            &RunOptions {
                timeout: Duration::from_millis(300),
                cancel: None,
                low_priority: true,
            },
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            AiError::Timeout {
                tool: "whisper-cli",
                ..
            }
        ),
        "{err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "process must be killed, not awaited"
    );

    // 취소
    let token = CancelToken::new();
    let t2 = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        t2.cancel();
    });
    let started = std::time::Instant::now();
    let err = ProcessRunner
        .run(
            Tool::Ffmpeg,
            &prog,
            &args,
            &RunOptions {
                timeout: Duration::from_secs(60),
                cancel: Some(token),
                low_priority: false,
            },
        )
        .unwrap_err();
    assert!(
        matches!(err, AiError::Cancelled { tool: "ffmpeg" }),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[cfg(windows)]
#[test]
fn t_ai_10_process_runner_reports_exit_code_stderr_and_spawn_failure() {
    // 비정상 종료 코드와 stderr 를 돌려준다 (`cmd /c exit 7`)
    let out = ProcessRunner
        .run(
            Tool::Ffmpeg,
            Path::new("cmd"),
            &["/c".into(), "echo oops 1>&2 & exit 7".into()],
            &RunOptions {
                timeout: Duration::from_secs(10),
                cancel: None,
                low_priority: false,
            },
        )
        .unwrap();
    assert_eq!(out.code, Some(7));
    assert!(out.stderr.contains("oops"));
    // 실행 파일이 없으면 I/O 오류
    let err = ProcessRunner
        .run(
            Tool::Ffmpeg,
            Path::new("definitely-not-a-real-binary-xyz.exe"),
            &[],
            &RunOptions {
                timeout: Duration::from_secs(1),
                cancel: None,
                low_priority: false,
            },
        )
        .unwrap_err();
    assert!(matches!(err, AiError::Io { .. }));
}

// ---------- 실제 whisper e2e (도구·모델·음성이 있을 때만) ----------

#[test]
fn t_ai_09_real_whisper_end_to_end_when_available() {
    let (Some(models), Some(whisper), Some(wav)) = (
        common::real_models(),
        std::env::var_os("MAM_AI_TEST_WHISPER").map(PathBuf::from),
        std::env::var_os("MAM_AI_TEST_STT_WAV").map(PathBuf::from),
    ) else {
        println!(
            "SKIP t_ai_09_real_whisper: MAM_AI_TEST_MODELS / MAM_AI_TEST_WHISPER / MAM_AI_TEST_STT_WAV not set"
        );
        return;
    };
    let ffmpeg = match resolve_tool(Tool::Ffmpeg, None, None, None) {
        Ok(p) => p,
        Err(_) => {
            println!("SKIP t_ai_09_real_whisper: ffmpeg not on PATH");
            return;
        }
    };
    let work = tempfile::tempdir().unwrap();
    let eng = SttEngine::new(
        Arc::new(ProcessRunner),
        SttPaths {
            whisper_cli: whisper,
            ffmpeg,
            model: models.join("whisper-base/model.bin"),
        },
        SttConfig {
            threads: 4,
            ..Default::default()
        },
    );
    let t = eng.transcribe(&wav, work.path(), None).expect("transcribe");
    println!("language={} text={}", t.language, t.full_text());
    assert_eq!(t.language, "ko");
    assert!(t.full_text().contains("회의"), "{}", t.full_text());
    assert!(entries(work.path()).is_empty());
}
