use std::path::PathBuf;
use std::time::Duration;

/// 미디어 처리 오류. 모든 오류는 **자산 단위**로 격리된다 — 한 파일의 실패가 다른 파일 처리를 막지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// FFmpeg/FFprobe 실행 파일을 찾지 못했다 — 기능 저하(degraded) 모드: 영상·음성 메타데이터와
    /// 대표 프레임만 불가하고 이미지 처리는 계속 동작한다.
    #[error("{tool} executable not found ({searched})")]
    ToolNotFound {
        tool: &'static str,
        searched: String,
    },

    /// 사이드카 프로세스가 제한 시간 안에 끝나지 않아 강제 종료했다.
    #[error("{tool} timed out after {timeout:?} and was killed")]
    Timeout {
        tool: &'static str,
        timeout: Duration,
    },

    /// 앱 종료 신호로 사이드카를 강제 종료했다 (실패가 아니다 - 작업은 다음 실행에서 이어진다).
    #[error("{tool} was cancelled and killed")]
    Cancelled { tool: &'static str },

    /// 사이드카 출력이 상한을 넘어 강제 종료했다.
    #[error("{tool} output exceeded {limit} bytes and was killed")]
    OutputTooLarge { tool: &'static str, limit: usize },

    /// 사이드카가 0 이 아닌 코드로 종료했다. `stderr` 는 앞부분만 보관한다.
    #[error("{tool} exited with code {code:?}: {stderr}")]
    ToolFailed {
        tool: &'static str,
        code: Option<i32>,
        stderr: String,
    },

    /// 프로세스 생성·파일 읽기/쓰기 등 I/O 오류.
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// ffprobe JSON 이 깨졌거나 필요한 정보가 없다.
    #[error("invalid probe output: {0}")]
    InvalidOutput(String),

    /// 이미지 디코딩 실패 (손상·미지원 형식).
    #[error("image decode failed: {0}")]
    Decode(String),

    /// 디코딩 메모리 상한 초과 — 전체 디코딩 전에 거부했다.
    #[error("image too large: {width}x{height} exceeds {max_pixels} pixels")]
    ImageTooLarge {
        width: u32,
        height: u32,
        max_pixels: u64,
    },

    /// RAW 파일에서 내장 JPEG 프리뷰를 찾지 못했다.
    #[error("no embedded preview found in raw file")]
    NoRawPreview,

    /// 이 종류의 미디어는 썸네일을 만들 수 없다 (음성, 문서 등).
    #[error("no thumbnail for this media: {0}")]
    Unsupported(&'static str),

    /// 썸네일 인코딩 실패.
    #[error("thumbnail encode failed: {0}")]
    Encode(String),
}

pub type Result<T> = std::result::Result<T, MediaError>;

impl MediaError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
