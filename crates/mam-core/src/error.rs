use std::path::PathBuf;

/// 자산 식별·reconcile 계층의 오류. 업무 규칙 위반은 별도 변형으로 구분한다.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a regular file: {}", .0.display())]
    NotAFile(PathBuf),
    /// OS 볼륨 API 실패 (API 이름 포함)
    #[error("volume API error: {0}")]
    Volume(String),
    #[error("path is not under the volume mount point: {path} (mount {mount})")]
    OutsideVolume { path: String, mount: String },
    /// 해시 도중 취소 플래그가 켜졌다 — 부분 결과는 기록하지 않는다 (D5)
    #[error("hashing cancelled")]
    Cancelled,
    /// 해시 전후 크기·수정 시각이 달라졌다 — 결과 폐기 (D5)
    #[error("file changed while hashing: {}", .0.display())]
    ChangedDuringHash(PathBuf),
    /// 대소문자만 다른 경로가 같은 볼륨에 공존 (D6) — 데이터는 변경하지 않았다
    #[error("case-only path collision on volume {volume_id}: {rel_path} vs {existing_rel_path}")]
    CaseCollision {
        volume_id: i64,
        rel_path: String,
        existing_rel_path: String,
    },
    #[error("volume {0} not found")]
    VolumeNotFound(i64),
    #[error("invalid relative path: {0}")]
    PathKey(#[from] mam_db::PathKeyError),
    #[error("internal error: {0}")]
    Internal(String),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

impl CoreError {
    pub(crate) fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
