// Categories: Happy (전사 저장 형태) / Business rule (모델·도구 없음은 건너뜀, 원본 비변경) / External (타임아웃·도구 실패·취소)
use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mam_ai::local::stt::{CommandOutput, RunOptions};
use mam_ai::models::Manifest;

use super::super::test_support::Fx;
use super::*;

const WHISPER_JSON: &str = r#"{"result":{"language":"ko"},"transcription":[
  {"offsets":{"from":0,"to":1500},"text":" 안녕하세요"},
  {"offsets":{"from":1500,"to":3000},"text":" 테스트입니다"}]}"#;

/// 가짜 ffmpeg/whisper: 출력 파일만 쓰고 정해진 동작을 한다.
type WhisperFn = Box<dyn Fn(&RunOptions) -> mam_ai::Result<CommandOutput> + Send + Sync>;

struct FakeRunner {
    whisper: WhisperFn,
    seen_timeouts: std::sync::Mutex<Vec<Duration>>,
}

fn ok_out() -> mam_ai::Result<CommandOutput> {
    Ok(CommandOutput {
        code: Some(0),
        stdout: String::new(),
        stderr: String::new(),
    })
}

impl CommandRunner for FakeRunner {
    fn run(
        &self,
        tool: Tool,
        _program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> mam_ai::Result<CommandOutput> {
        self.seen_timeouts.lock().unwrap().push(opts.timeout);
        match tool {
            Tool::Ffmpeg => {
                let wav = args.last().unwrap();
                std::fs::write(wav, vec![0u8; 44 + 32_000]).unwrap();
                ok_out()
            }
            Tool::WhisperCli => {
                let of = args.iter().position(|a| a == "-of").unwrap();
                let base = Path::new(&args[of + 1]).to_path_buf();
                std::fs::write(base.with_extension("json"), WHISPER_JSON).unwrap();
                (self.whisper)(opts)
            }
        }
    }
}

fn tiny_models(dir: &Path, install: bool) -> Arc<ModelStore> {
    let json = r#"{"version":1,"models":[
      {"id":"whisper-base","kind":"stt","display_name":"w","license":"MIT","license_url":"https://x","source_url":"https://x",
       "files":[{"path":"model.bin","url":"https://example.invalid/m","size":4,
                 "sha256":"0000000000000000000000000000000000000000000000000000000000000000"}]}]}"#;
    let root = dir.join("models");
    if install {
        std::fs::create_dir_all(root.join("whisper-base")).unwrap();
        std::fs::write(root.join("whisper-base").join("model.bin"), b"abcd").unwrap();
    }
    Arc::new(ModelStore::new(root, Manifest::parse(json).unwrap()))
}

fn stt(fx: &Fx, install: bool, runner: Arc<FakeRunner>, tools_ok: bool) -> LocalStt {
    LocalStt::new(
        Arc::clone(&fx.service.store),
        tiny_models(fx.dir.path(), install),
        runner,
        Arc::new(move || {
            if tools_ok {
                Ok((
                    PathBuf::from("whisper-cli.exe"),
                    PathBuf::from("ffmpeg.exe"),
                ))
            } else {
                Err(AiError::ToolNotFound {
                    tool: "whisper-cli",
                    searched: "t".into(),
                })
            }
        }),
        2,
        fx.dir.path().join("work"),
    )
}

fn runner(
    f: impl Fn(&RunOptions) -> mam_ai::Result<CommandOutput> + Send + Sync + 'static,
) -> Arc<FakeRunner> {
    Arc::new(FakeRunner {
        whisper: Box::new(f),
        seen_timeouts: Default::default(),
    })
}

fn req(fx: &Fx, timeout_secs: u64) -> (SttRequest, Vec<u8>) {
    let media = fx.dir.path().join("talk.mp3");
    std::fs::write(&media, b"ORIGINAL-AUDIO").unwrap();
    (
        SttRequest {
            asset_id: 1,
            media_path: media,
            duration_ms: Some(3000),
            timeout: Duration::from_secs(timeout_secs),
        },
        b"ORIGINAL-AUDIO".to_vec(),
    )
}

#[test]
fn t_p3w_stt_01_transcript_is_mapped_and_original_untouched() {
    // Arrange
    let fx = Fx::new();
    let (request, original) = req(&fx, 90);
    let r = runner(|_| ok_out());
    let adapter = stt(&fx, true, Arc::clone(&r), true);
    // Act
    let rec = adapter.transcribe(&request, &|| false).unwrap();
    // Assert
    assert_eq!(rec.language, "ko");
    assert_eq!(rec.segments.len(), 2);
    assert_eq!(
        (rec.segments[1].start_ms, rec.segments[1].end_ms),
        (1500, 3000)
    );
    assert_eq!(rec.engine, "whisper.cpp");
    assert_eq!(rec.model, "whisper-base");
    assert_eq!(
        std::fs::read(&request.media_path).unwrap(),
        original,
        "원본 보호"
    );
    // 코어가 정한 제한 시간이 whisper 실행에 그대로 쓰인다
    assert!(
        r.seen_timeouts
            .lock()
            .unwrap()
            .contains(&Duration::from_secs(90))
    );
    // 작업 폴더는 정리된다
    let leftovers = std::fs::read_dir(fx.dir.path().join("work"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}

#[test]
fn t_p3w_stt_02_missing_model_or_tool_is_skip_codes() {
    let fx = Fx::new();
    let (request, _) = req(&fx, 60);
    let e = stt(&fx, false, runner(|_| ok_out()), true)
        .transcribe(&request, &|| false)
        .unwrap_err();
    assert_eq!(e.code, "model_missing");
    let e = stt(&fx, true, runner(|_| ok_out()), false)
        .transcribe(&request, &|| false)
        .unwrap_err();
    assert_eq!(e.code, "unsupported");
}

#[test]
fn t_p3w_stt_03_tool_errors_map_without_leaking_stderr() {
    let fx = Fx::new();
    let (request, _) = req(&fx, 60);
    let e = stt(
        &fx,
        true,
        runner(|_| {
            Err(AiError::ToolFailed {
                tool: "whisper-cli",
                code: Some(3),
                stderr: r"C:\Users\me\secret.wav broken".into(),
            })
        }),
        true,
    )
    .transcribe(&request, &|| false)
    .unwrap_err();
    assert_eq!(e.code, "stt_failed");
    assert!(!e.message.contains("secret"));
    let e = stt(
        &fx,
        true,
        runner(|_| {
            Err(AiError::Timeout {
                tool: "whisper-cli",
                timeout: Duration::from_secs(1),
            })
        }),
        true,
    )
    .transcribe(&request, &|| false)
    .unwrap_err();
    assert_eq!(e.code, "stt_timeout");
}

#[test]
fn t_p3w_stt_04_cancel_signal_reaches_the_tool_run() {
    // Arrange: whisper 가 취소 토큰이 켜질 때까지 기다린다
    let fx = Fx::new();
    let (request, _) = req(&fx, 60);
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let r = runner(move |opts| {
        let token = opts.cancel.clone().unwrap();
        for _ in 0..200 {
            if token.is_cancelled() {
                return Err(AiError::Cancelled {
                    tool: "whisper-cli",
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        flag.store(true, Ordering::SeqCst); // 취소가 전달되지 않았다
        ok_out()
    });
    let adapter = stt(&fx, true, r, true);
    // Act
    let e = adapter.transcribe(&request, &|| true).unwrap_err();
    // Assert
    assert_eq!(e.code, "cancelled");
    assert!(!cancelled.load(Ordering::SeqCst));
}
