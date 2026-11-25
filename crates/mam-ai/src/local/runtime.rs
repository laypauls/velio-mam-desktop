//! ONNX Runtime 환경·세션 생성 — 프로세스에 환경 1개를 공유한다.

use std::path::Path;
use std::sync::OnceLock;

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;

use crate::error::{AiError, Result};

fn env() -> Result<&'static Environment> {
    static ENV: OnceLock<std::result::Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| ort::init().build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| AiError::Onnx(e.clone()))
}

/// CPU 실행 공급자로 세션을 만든다. `threads` 는 연산 내부 병렬 스레드 수 (1 이상).
///
/// 모델 파일은 읽기만 한다. GPU 공급자는 쓰지 않는다(CUDA 런타임 배포 부담 — P1-6 범위 밖).
pub(crate) fn build_session(model: &Path, threads: usize) -> Result<Session> {
    if !model.is_file() {
        return Err(AiError::ModelMissing {
            model_id: model
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            file: model.display().to_string(),
        });
    }
    let mut builder = Session::builder(env()?)?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads.max(1))?
        // 배치 하나를 순차 실행 — 병렬 그래프 실행은 메모리만 늘린다.
        .with_parallel_execution(false)?;
    Ok(builder.commit_from_file(model)?)
}
