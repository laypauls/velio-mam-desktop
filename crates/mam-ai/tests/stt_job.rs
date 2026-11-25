//! T-stt-01~03 - 자산 전사 작업 함수. 분류: Happy / External / Business rule(원본 보호).
//! ffmpeg·whisper 는 `CommandRunner` 스텁이다 (모델·바이너리 없는 CI 에서도 동작).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mam_ai::AiError;
use mam_ai::local::stt::{
    CancelToken, CommandOutput, CommandRunner, RunOptions, SttConfig, SttEngine, SttPaths, Tool,
    transcribe_asset,
};

const JSON: &str = r#"{
 "result": {"language": "ko"},
 "transcription": [
  {"offsets": {"from": 0, "to": 2800}, "text": " 오늘 meeting 은"},
  {"offsets": {"from": 2800, "to": 9100}, "text": " 일정을 논의했습니다."}
 ]
}"#;

struct Stub {
    whisper_code: i32,
    cancel_in_ffmpeg: bool,
}

impl CommandRunner for Stub {
    fn run(
        &self,
        tool: Tool,
        _p: &Path,
        args: &[OsString],
        _o: &RunOptions,
    ) -> Result<CommandOutput, AiError> {
        let mut code = 0;
        match tool {
            Tool::Ffmpeg => {
                if self.cancel_in_ffmpeg {
                    return Err(AiError::Cancelled { tool: "ffmpeg" });
                }
                std::fs::write(args.last().unwrap(), vec![0u8; 44 + 32_000 * 10]).unwrap();
            }
            Tool::WhisperCli => {
                let i = args.iter().position(|a| a == "-of").unwrap();
                let base = PathBuf::from(&args[i + 1]);
                std::fs::write(format!("{}.json", base.display()), JSON).unwrap();
                code = self.whisper_code;
            }
        }
        Ok(CommandOutput {
            code: Some(code),
            stdout: String::new(),
            stderr: if code == 0 {
                String::new()
            } else {
                "boom".into()
            },
        })
    }
}

fn engine(stub: Stub) -> SttEngine {
    SttEngine::new(
        Arc::new(stub),
        SttPaths {
            whisper_cli: "whisper-cli.exe".into(),
            ffmpeg: "ffmpeg.exe".into(),
            model: PathBuf::from("models/whisper-base/model.bin"),
        },
        SttConfig::default(),
    )
}

fn names(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

fn ok_stub() -> Stub {
    Stub {
        whisper_code: 0,
        cancel_in_ffmpeg: false,
    }
}

#[test]
fn t_stt_01_returns_language_segments_engine_model_and_duration_and_cleans_work_dir() {
    // Arrange
    let media = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = media.path().join("call.m4a");
    std::fs::write(&input, b"audio").unwrap();

    // Act
    let r = transcribe_asset(&engine(ok_stub()), &input, work.path(), None).unwrap();

    // Assert
    assert_eq!(r.language, "ko");
    assert_eq!(r.segments.len(), 2);
    assert_eq!(r.segments[0].text, "오늘 meeting 은");
    assert_eq!(r.engine_id, "whisper.cpp");
    assert_eq!(r.model_id, "whisper-base");
    assert_eq!(r.duration_ms, 9100);
    assert!(names(work.path()).is_empty(), "{:?}", names(work.path()));
}

#[test]
fn t_stt_02_whisper_failure_and_cancel_return_errors_and_clean_work_dir() {
    let media = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = media.path().join("a.wav");
    std::fs::write(&input, b"x").unwrap();

    let e = transcribe_asset(
        &engine(Stub {
            whisper_code: 3,
            cancel_in_ffmpeg: false,
        }),
        &input,
        work.path(),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(
            e,
            AiError::ToolFailed {
                tool: "whisper-cli",
                code: Some(3),
                ..
            }
        ),
        "{e}"
    );
    assert!(names(work.path()).is_empty());

    let token = CancelToken::new();
    token.cancel();
    let e = transcribe_asset(
        &engine(Stub {
            whisper_code: 0,
            cancel_in_ffmpeg: true,
        }),
        &input,
        work.path(),
        Some(&token),
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Cancelled { .. }), "{e}");
    assert!(names(work.path()).is_empty());

    // 입력 파일이 없으면 I/O 오류 (작업 폴더를 만들지 않는다)
    let e = transcribe_asset(
        &engine(ok_stub()),
        &media.path().join("none.mp3"),
        work.path(),
        None,
    )
    .unwrap_err();
    assert!(matches!(e, AiError::Io { .. }));
    assert!(names(work.path()).is_empty());
}

#[test]
fn t_stt_03_source_is_untouched_and_nothing_is_written_beside_it() {
    let media = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = media.path().join("interview.mp3");
    std::fs::write(&input, b"original audio bytes").unwrap();
    let before = (
        std::fs::read(&input).unwrap(),
        std::fs::metadata(&input).unwrap().modified().unwrap(),
    );

    transcribe_asset(&engine(ok_stub()), &input, work.path(), None).unwrap();
    let _ = transcribe_asset(
        &engine(Stub {
            whisper_code: 1,
            cancel_in_ffmpeg: false,
        }),
        &input,
        work.path(),
        None,
    );

    assert_eq!(names(media.path()), vec![input.clone()]);
    assert_eq!(
        (
            std::fs::read(&input).unwrap(),
            std::fs::metadata(&input).unwrap().modified().unwrap()
        ),
        before
    );
}
