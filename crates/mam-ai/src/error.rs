use std::path::PathBuf;
use std::time::Duration;

/// 로컬 AI 오류. 모든 오류는 **작업 단위**로 격리된다 — 한 파일·한 모델의 실패가 다른 기능을 막지 않는다.
/// 오류 메시지에는 API 키·토큰·사용자 파일 내용을 넣지 않는다 (경로와 상태 코드만).
#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// 네트워크에 연결하지 못했다 — 오프라인. 모델 다운로드만 실패하고 이미 받은 모델은 계속 쓸 수 있다.
    #[error("offline: cannot reach {host}")]
    Offline { host: String },

    /// 서버가 오류 상태를 돌려줬다 (404 등).
    #[error("download failed: {url} returned HTTP {status}")]
    Http { url: String, status: u16 },

    /// 연결은 됐으나 전송 중 끊겼거나 길이가 맞지 않는다. 부분 파일은 남기므로 다시 시도하면 이어받는다.
    #[error("download interrupted: {received} of {expected} bytes of {file}")]
    Interrupted {
        file: String,
        received: u64,
        expected: u64,
    },

    /// 다운로드한 파일의 SHA-256 이 매니페스트와 다르다. 부분 파일은 삭제했다.
    #[error("sha256 mismatch for {file}: expected {expected}, got {actual}")]
    HashMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    /// 모델 저장 폴더의 여유 공간이 부족하다.
    #[error("not enough disk space in {dir}: need {needed} bytes, {available} available")]
    InsufficientSpace {
        dir: PathBuf,
        needed: u64,
        available: u64,
    },

    /// 매니페스트에 없는 모델 ID.
    #[error("unknown model id: {0}")]
    UnknownModel(String),

    /// 매니페스트가 깨졌거나 규칙(상대 경로, SHA-256 형식 등)을 어겼다.
    #[error("invalid manifest: {0}")]
    Manifest(String),

    /// 모델 파일이 아직 없다 (다운로드 전).
    #[error("model {model_id} is not downloaded (missing {file})")]
    ModelMissing { model_id: String, file: String },

    /// ONNX Runtime 오류.
    #[error("onnx runtime error: {0}")]
    Onnx(String),

    /// 모델 출력 형태가 기대와 다르다 (차원, 출력 이름).
    #[error("unexpected model output: {0}")]
    ModelOutput(String),

    /// 토크나이저 오류.
    #[error("tokenizer error: {0}")]
    Tokenizer(String),

    /// 입력 검증 실패 (빈 텍스트, 빈 배치 등).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// 이미지 디코딩 실패 (손상·미지원 형식·크기 초과).
    #[error("image decode failed: {0}")]
    Decode(String),

    /// 외부 도구(whisper-cli, ffmpeg)를 찾지 못했다 — STT 만 비활성이다.
    #[error("{tool} executable not found ({searched})")]
    ToolNotFound {
        tool: &'static str,
        searched: String,
    },

    /// 외부 도구가 제한 시간 안에 끝나지 않아 강제 종료했다.
    #[error("{tool} timed out after {timeout:?} and was killed")]
    Timeout {
        tool: &'static str,
        timeout: Duration,
    },

    /// 호출자가 취소해 외부 도구를 강제 종료했다.
    #[error("{tool} was cancelled and killed")]
    Cancelled { tool: &'static str },

    /// 외부 도구가 0 이 아닌 코드로 종료했다. `stderr` 는 앞부분만 보관한다.
    #[error("{tool} exited with code {code:?}: {stderr}")]
    ToolFailed {
        tool: &'static str,
        code: Option<i32>,
        stderr: String,
    },

    /// 외부 도구 출력(JSON)이 깨졌다.
    #[error("invalid {tool} output: {reason}")]
    ToolOutput { tool: &'static str, reason: String },

    /// 파일 읽기/쓰기·프로세스 생성 I/O 오류.
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, AiError>;

impl AiError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl<R> From<ort::Error<R>> for AiError {
    fn from(e: ort::Error<R>) -> Self {
        Self::Onnx(e.to_string())
    }
}
