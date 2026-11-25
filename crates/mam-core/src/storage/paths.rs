//! MAMData 하위 구조 (기술 검토서 §9). 경로는 모두 `root` 에서 파생한다.

use std::path::{Path, PathBuf};

use super::StorageError;

/// MAMData 안의 고정 경로. 업데이트 안전장치(`update_guard`)가 쓰는 경로도 여기서 정의해 한 곳에서 관리한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    root: PathBuf,
}

impl DataPaths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            root: data_dir.into(),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn database_dir(&self) -> PathBuf {
        self.root.join("database")
    }
    pub fn db(&self) -> PathBuf {
        self.database_dir().join("mam.sqlite")
    }
    pub fn tantivy(&self) -> PathBuf {
        self.root.join("indexes").join("tantivy")
    }
    pub fn vectors_visual(&self) -> PathBuf {
        self.root.join("indexes").join("vectors").join("visual")
    }
    pub fn vectors_semantic(&self) -> PathBuf {
        self.root.join("indexes").join("vectors").join("semantic")
    }
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
    pub fn thumbnails(&self) -> PathBuf {
        self.cache().join("thumbnails")
    }
    pub fn previews(&self) -> PathBuf {
        self.cache().join("previews")
    }
    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }
    /// 설치된 플러그인 패키지 (`plugins/<id>/<version>/`) — 사용자 폴더에서 복사한 사본이며 MAM 산출물이다
    pub fn plugins(&self) -> PathBuf {
        self.root.join("plugins")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn updates(&self) -> PathBuf {
        self.root.join("updates")
    }
    pub fn tmp(&self) -> PathBuf {
        self.root.join("tmp")
    }
    /// 단일 인스턴스 잠금 파일.
    pub fn lock_file(&self) -> PathBuf {
        self.root.join(".lock")
    }
    pub fn journal(&self) -> PathBuf {
        self.root.join("update-journal.json")
    }
    /// 지금 실행 중인(검증된) 버전의 설치기. 다음 업데이트의 "이전 설치기"가 된다.
    pub fn current_installer(&self) -> PathBuf {
        self.updates().join("current-setup.exe")
    }
    /// NSIS 훅이 설치 직후 복사해 두는 설치기 — 새 버전이 `healthy` 가 되면 `current` 로 승격한다.
    pub fn pending_installer(&self) -> PathBuf {
        self.updates().join("pending-setup.exe")
    }
    /// 업데이트 진행 중 표지 — NSIS 훅이 이 파일이 있을 때만 사후 검증·복구를 한다.
    pub fn in_progress_marker(&self) -> PathBuf {
        self.updates().join("update-in-progress")
    }
    /// NSIS 훅이 사후 검증 실패로 이전 설치기를 실행했다는 표지 — 복구된 구 버전이 DB 를 되돌린다.
    pub fn rollback_flag(&self) -> PathBuf {
        self.updates().join("rollback-requested")
    }

    /// 만들어야 하는 디렉터리 목록 (`.lock`·journal 같은 파일은 제외).
    fn directories(&self) -> Vec<PathBuf> {
        vec![
            self.database_dir(),
            self.tantivy(),
            self.vectors_visual(),
            self.vectors_semantic(),
            self.thumbnails(),
            self.previews(),
            self.models().join("embedding"),
            self.models().join("stt"),
            self.plugins(),
            self.backups(),
            self.logs(),
            self.updates(),
            self.tmp(),
        ]
    }

    /// 하위 구조를 만든다(멱등). 마지막에 `tmp/` 에 쓰기 탐침 파일을 만들었다 지워 쓰기 가능 여부를 확인한다.
    pub fn ensure_layout(&self) -> Result<(), StorageError> {
        for dir in self.directories() {
            std::fs::create_dir_all(&dir).map_err(|e| StorageError::io(&dir, e))?;
        }
        let probe = self.tmp().join(".write-probe");
        std::fs::write(&probe, b"").map_err(|_| StorageError::NotWritable(self.root.clone()))?;
        let _ = std::fs::remove_file(&probe);
        Ok(())
    }
}
