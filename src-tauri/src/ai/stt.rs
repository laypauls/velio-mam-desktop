//! 로컬 STT 어댑터 — 작업 코어의 `SttRunner` 를 `mam_ai::local::stt::transcribe_asset` 위에 구현한다 (P3-4).
//!
//! 원본 음성은 외부로 보내지 않는다(PA5). 임시 WAV 는 `MAMData/cache/tmp` 아래에만 쓰고 끝나면 지워진다.
//! 도구(whisper-cli·ffmpeg)나 모델이 없으면 실패가 아니라 건너뜀(`unsupported`/`model_missing`)이다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mam_ai::AiError;
use mam_ai::local::stt::{
    CancelToken, CommandRunner, Language, SttConfig, SttEngine, SttPaths, Tool, transcribe_asset,
};
use mam_ai::models::{ModelStatus, ModelStore};
use mam_core::ai_jobs::{StepFailure, SttRequest, SttRunner};
use mam_core::ai_store::{TranscriptRecord, TranscriptSegment};

use super::store::AiStore;

/// 실행 파일 위치 해석 (시험에서 가짜로 바꾼다).
pub type ToolResolver = dyn Fn() -> Result<(PathBuf, PathBuf), AiError> + Send + Sync;

pub struct LocalStt {
    store: Arc<AiStore>,
    models: Arc<ModelStore>,
    runner: Arc<dyn CommandRunner>,
    tools: Arc<ToolResolver>,
    threads: usize,
    work_dir: PathBuf,
}

/// whisper-cli·ffmpeg 를 사이드카(실행 파일 옆) -> PATH 순으로 찾는다.
pub fn system_tools() -> Result<(PathBuf, PathBuf), AiError> {
    let sidecar = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from));
    let find = |t| mam_ai::local::stt::resolve_tool(t, None, sidecar.as_deref(), None);
    Ok((find(Tool::WhisperCli)?, find(Tool::Ffmpeg)?))
}

impl LocalStt {
    pub fn new(
        store: Arc<AiStore>,
        models: Arc<ModelStore>,
        runner: Arc<dyn CommandRunner>,
        tools: Arc<ToolResolver>,
        threads: usize,
        work_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            models,
            runner,
            tools,
            threads,
            work_dir,
        }
    }
}

fn map_err(e: &AiError) -> StepFailure {
    match e {
        AiError::ToolNotFound { .. } => StepFailure::new(
            "unsupported",
            "음성 인식 도구(whisper-cli/ffmpeg)를 찾을 수 없습니다",
        ),
        AiError::ModelMissing { .. } => {
            StepFailure::new("model_missing", "음성 인식 모델이 설치되지 않았습니다")
        }
        AiError::Timeout { .. } => {
            StepFailure::new("stt_timeout", "음성 인식이 제한 시간을 넘겨 중단되었습니다")
        }
        AiError::Cancelled { .. } => StepFailure::new("cancelled", "중단되었습니다"),
        AiError::Io { .. } => StepFailure::new("io_transient", "파일을 읽거나 쓰지 못했습니다"),
        // 도구 stderr 에는 경로가 섞일 수 있어 사용자 메시지에 싣지 않는다
        _ => StepFailure::new("stt_failed", "음성 인식 도구가 오류로 종료되었습니다"),
    }
}

impl SttRunner for LocalStt {
    fn transcribe(
        &self,
        req: &SttRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<TranscriptRecord, StepFailure> {
        let model_id = self.store.settings().local_stt.model.model_id();
        let model_file = match self.models.status(model_id) {
            Ok(ModelStatus::Ready(p)) => p
                .file("model.bin")
                .map(PathBuf::from)
                .map_err(|e| map_err(&e))?,
            Ok(_) => {
                return Err(StepFailure::new(
                    "model_missing",
                    "음성 인식 모델이 설치되지 않았습니다",
                ));
            }
            Err(e) => return Err(map_err(&e)),
        };
        let (whisper_cli, ffmpeg) = (self.tools)().map_err(|e| map_err(&e))?;
        let cfg = SttConfig {
            threads: self.threads,
            language: Language::Auto,
            // 제한 시간은 코어가 자산별로 계산해 넘긴다 (길이 비례 항은 끈다)
            timeout_floor: req.timeout,
            timeout_rtf: 0.0,
            convert_timeout: req.timeout.min(Duration::from_secs(300)),
            low_priority: true,
        };
        let engine = SttEngine::new(
            Arc::clone(&self.runner),
            SttPaths {
                whisper_cli,
                ffmpeg,
                model: model_file,
            },
            cfg,
        );
        let token = CancelToken::new();
        let result = std::thread::scope(|s| {
            let worker = s
                .spawn(|| transcribe_asset(&engine, &req.media_path, &self.work_dir, Some(&token)));
            // 코어의 중단 신호(Sync 가 아니라 이 스레드에서만 본다)를 도구 프로세스 종료로 전달한다
            while !worker.is_finished() {
                if cancel() {
                    token.cancel();
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            worker.join()
        });
        let t = result
            .map_err(|_| StepFailure::new("stt_failed", "음성 인식 중 오류가 발생했습니다"))?
            .map_err(|e| map_err(&e))?;
        Ok(TranscriptRecord {
            language: t.language,
            segments: t
                .segments
                .into_iter()
                .map(|s| TranscriptSegment {
                    start_ms: s.start_ms,
                    end_ms: s.end_ms,
                    text: s.text,
                })
                .collect(),
            engine: t.engine_id,
            model: t.model_id,
        })
    }
}

#[cfg(test)]
#[path = "stt_tests.rs"]
mod tests;
