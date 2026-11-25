/// `mam-vector` 의 오류. 손상 파일·메타 불일치·읽기 전용 위반은 모두 이 타입으로 반환하며 패닉하지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum VectorError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// 호출자가 넘긴 값이 규칙에 어긋난다 (k=0, NaN/Inf, 영벡터 등).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("dimension mismatch: index has {expected}, got {found}")]
    DimensionMismatch { expected: usize, found: usize },

    /// 디렉터리의 인덱스가 다른 엔진·버전·모델·차원·양자화로 만들어졌다. 삭제 후 재구축한다.
    #[error("index was built with different settings (rebuild required): {0}")]
    IndexMismatch(String),

    /// 인덱스 파일이 잘렸거나 헤더/크기가 맞지 않는다. 삭제 후 원시 임베딩에서 재구축한다.
    #[error("index file is corrupt: {0}")]
    Corrupt(String),

    /// view(메모리 매핑) 로 연 인덱스에는 쓸 수 없다.
    #[error("index is opened read-only (memory-mapped view)")]
    ReadOnly,

    /// 해당 엔진이 이 기능을 지원하지 않는다 (예: 파일 기반 view 가 없는 엔진).
    #[error("unsupported by engine: {0}")]
    Unsupported(&'static str),

    /// cargo feature 가 꺼져 있어 엔진을 쓸 수 없다.
    #[error("engine '{0}' is not compiled in (enable the cargo feature)")]
    EngineUnavailable(&'static str),

    /// 엔진이 반환한 오류.
    #[error("engine error: {0}")]
    Engine(String),
}

pub type Result<T> = std::result::Result<T, VectorError>;
