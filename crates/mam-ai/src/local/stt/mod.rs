//! 로컬 STT (D4) - FFmpeg 로 16kHz mono WAV 변환 -> whisper.cpp `whisper-cli` 사이드카 실행 -> JSON 파싱.
//!
//! 원본 음성은 **읽기 전용**으로만 쓰인다(FFmpeg `-i`). 변환 WAV 와 JSON 은 호출자가 넘긴 MAM 임시 폴더
//! (`work_dir`) 아래 작업별 하위 폴더에만 쓰며 성공·실패·취소와 무관하게 종료 시 삭제한다.
//! 원본 음성을 외부 서비스로 보내지 않는다 (정책 미정 - 로컬 엔진만).

pub mod job;
mod parse;
mod proc_tree;
mod tool;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use job::{TranscriptResult, transcribe_asset};
pub use parse::{Segment, Transcript, parse_whisper_json};
pub use tool::{
    CancelToken, CommandOutput, CommandRunner, ProcessRunner, RunOptions, Tool, resolve_tool,
};

use crate::error::{AiError, Result};

/// 16kHz mono s16le WAV 의 초당 바이트 수.
const WAV_BYTES_PER_SEC: u64 = 16_000 * 2;
const WAV_HEADER: u64 = 44;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Language {
    /// 자동 감지 (`-l auto`). 한국어·영어 혼합 라이브러리의 기본값.
    Auto,
    /// 고정 (`ko`, `en`) - 감지 오류를 피하고 속도도 약간 빠르다.
    Fixed(String),
}

#[derive(Debug, Clone)]
pub struct SttConfig {
    /// whisper 스레드 수.
    pub threads: usize,
    pub language: Language,
    /// 제한 시간 = max(`timeout_floor`, 오디오 길이 x `timeout_rtf`). 저사양 PC 의 느린 처리를 오류로 오인하지 않게
    /// 오디오 길이에 비례시킨다.
    pub timeout_floor: Duration,
    pub timeout_rtf: f64,
    /// FFmpeg 변환 제한 시간.
    pub convert_timeout: Duration,
    /// 낮은 프로세스 우선순위로 실행 (UI 반응 우선).
    pub low_priority: bool,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            language: Language::Auto,
            timeout_floor: Duration::from_secs(120),
            timeout_rtf: 6.0,
            convert_timeout: Duration::from_secs(300),
            low_priority: true,
        }
    }
}

/// STT 에 필요한 실행 파일·모델 경로.
#[derive(Debug, Clone)]
pub struct SttPaths {
    pub whisper_cli: PathBuf,
    pub ffmpeg: PathBuf,
    /// ggml 가중치 (`ggml-base.bin` 등).
    pub model: PathBuf,
}

pub struct SttEngine {
    runner: Arc<dyn CommandRunner>,
    paths: SttPaths,
    cfg: SttConfig,
}

/// 작업별 임시 폴더 - 드롭 시 삭제(성공·실패·패닉 모두).
struct JobDir(PathBuf);

impl JobDir {
    fn create(work_dir: &Path) -> Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "stt-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let dir = work_dir.join(name);
        fs::create_dir_all(&dir).map_err(|e| AiError::io(&dir, e))?;
        Ok(Self(dir))
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl SttEngine {
    pub fn new(runner: Arc<dyn CommandRunner>, paths: SttPaths, cfg: SttConfig) -> Self {
        Self { runner, paths, cfg }
    }

    /// `input`(오디오/영상 파일, 읽기 전용)을 전사한다. 임시 파일은 `work_dir` 아래에만 만든다.
    pub fn transcribe(
        &self,
        input: &Path,
        work_dir: &Path,
        cancel: Option<&CancelToken>,
    ) -> Result<Transcript> {
        fs::metadata(input).map_err(|e| AiError::io(input, e))?;
        let job = JobDir::create(work_dir)?;
        let wav = job.0.join("audio.wav");
        let out_base = job.0.join("out");

        // 1) 16kHz mono WAV 변환
        let ffmpeg_args: Vec<OsString> = ["-nostdin", "-v", "error", "-y", "-i"]
            .iter()
            .map(OsString::from)
            .chain([input.as_os_str().to_owned()])
            .chain(
                [
                    "-vn",
                    "-ac",
                    "1",
                    "-ar",
                    "16000",
                    "-c:a",
                    "pcm_s16le",
                    "-f",
                    "wav",
                ]
                .iter()
                .map(OsString::from),
            )
            .chain([wav.as_os_str().to_owned()])
            .collect();
        self.run_ok(
            Tool::Ffmpeg,
            &self.paths.ffmpeg,
            &ffmpeg_args,
            self.cfg.convert_timeout,
            cancel,
        )?;

        let wav_len = fs::metadata(&wav).map_err(|e| AiError::io(&wav, e))?.len();
        let seconds = wav_len.saturating_sub(WAV_HEADER) / WAV_BYTES_PER_SEC;
        if wav_len <= WAV_HEADER {
            // 오디오 샘플이 없다 - whisper 를 돌리지 않는다.
            return Ok(Transcript {
                language: String::new(),
                segments: Vec::new(),
            });
        }

        // 2) whisper-cli
        let lang = match &self.cfg.language {
            Language::Auto => "auto".to_string(),
            Language::Fixed(l) => l.clone(),
        };
        let args: Vec<OsString> = vec![
            "-m".into(),
            self.paths.model.as_os_str().to_owned(),
            "-f".into(),
            wav.as_os_str().to_owned(),
            "-l".into(),
            lang.into(),
            "-t".into(),
            self.cfg.threads.max(1).to_string().into(),
            "-oj".into(),
            "-of".into(),
            out_base.as_os_str().to_owned(),
            "-np".into(),
        ];
        let timeout = self.cfg.timeout_floor.max(Duration::from_secs_f64(
            seconds as f64 * self.cfg.timeout_rtf,
        ));
        self.run_ok(
            Tool::WhisperCli,
            &self.paths.whisper_cli,
            &args,
            timeout,
            cancel,
        )?;

        // 3) 결과 JSON
        let json_path = job.0.join("out.json");
        let json = fs::read_to_string(&json_path).map_err(|e| AiError::io(&json_path, e))?;
        parse_whisper_json(&json)
    }

    fn run_ok(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        timeout: Duration,
        cancel: Option<&CancelToken>,
    ) -> Result<()> {
        let opts = RunOptions {
            timeout,
            cancel: cancel.cloned(),
            low_priority: self.cfg.low_priority,
        };
        let out = self.runner.run(tool, program, args, &opts)?;
        if out.code != Some(0) {
            return Err(AiError::ToolFailed {
                tool: tool.name(),
                code: out.code,
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(())
    }
}
