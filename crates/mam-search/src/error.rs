/// `mam-search` 의 오류. 손상·잠금·사전 로드 실패는 모두 이 타입으로 반환하며 패닉하지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// Tantivy 오류 — 인덱스 손상(`DataCorruption`), 쓰기 잠금 중복(`LockFailure`),
    /// 스키마 불일치 등이 여기로 온다.
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("cannot open index directory: {0}")]
    OpenDirectory(#[from] tantivy::directory::error::OpenDirectoryError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Lindera ko-dic 사전/세그먼터 초기화 또는 분석 실패.
    #[error("korean dictionary error: {0}")]
    Dictionary(String),

    /// 디렉터리의 인덱스가 다른 형식·분석기 버전·전략 옵션으로 만들어졌다. 삭제 후 재구축한다.
    #[error("index was built with different settings (rebuild required): {0}")]
    IndexMismatch(String),

    /// 실패한 배치의 rollback 도 실패해 writer 를 믿을 수 없다. 인덱스를 다시 열어야 한다.
    #[error("index writer is poisoned after a failed rollback; reopen the index")]
    WriterPoisoned,

    /// 호출자가 넘긴 문서·질의 값이 규칙에 어긋난다 (예: revision 0).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// 검색할 인덱스가 아직 없다 (최초 구축 중이거나 재구축 교체가 실패했다).
    #[error("search index is not ready")]
    NotReady,

    /// 커서가 이전 인덱스 세대의 것이거나 요청과 맞지 않는다 — 첫 페이지부터 다시 요청한다.
    #[error("cursor is stale or does not match the request")]
    StaleCursor,
}

pub type Result<T> = std::result::Result<T, SearchError>;
