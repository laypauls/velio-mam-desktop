//! 모델 저장소 — `MAMData/models/<model_id>/<file.path>`.
//!
//! 최종 경로의 파일은 항상 **검증을 마친 완전한 파일**이다(`.part` 에 받고 검증 후 이동). 그래서 "준비됨" 판정은
//! 파일 존재 + 크기 비교만으로 충분하다. 전체 해시 재검증은 [`ModelStore::verify`] 로 따로 한다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::download::{download_file, sha256_file};
use super::manifest::{Manifest, ModelSpec};
use super::space::{SpaceProbe, SystemSpace};
use crate::error::{AiError, Result};

/// 준비된 모델의 파일 경로.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPaths {
    pub model_id: String,
    pub dir: PathBuf,
    files: Vec<(String, PathBuf)>,
}

impl ModelPaths {
    /// 매니페스트의 상대 경로(`onnx/model.onnx`)로 파일 경로를 얻는다.
    pub fn file(&self, rel: &str) -> Result<&Path> {
        self.files
            .iter()
            .find(|(r, _)| r == rel)
            .map(|(_, p)| p.as_path())
            .ok_or_else(|| AiError::ModelMissing {
                model_id: self.model_id.clone(),
                file: rel.to_string(),
            })
    }
}

/// 모델 준비 상태 (다운로드 없이 조회).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStatus {
    NotDownloaded,
    /// 일부 파일만 있다 (중단된 다운로드).
    Partial {
        have: usize,
        total: usize,
    },
    Ready(ModelPaths),
}

/// 기능 가용 상태. 다운로드에 실패하면 **해당 기능만** 비활성이고 호출자는 다른 기능을 계속 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureState {
    Ready(ModelPaths),
    Disabled(DisabledReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisabledReason {
    /// 네트워크 없음 — 모델 최초 다운로드 불가. 연결되면 다시 시도한다.
    Offline,
    /// 저장 공간 부족.
    DiskFull { needed: u64, available: u64 },
    /// 그 밖의 실패 (404, 해시 불일치, 쓰기 오류 등). 메시지에는 비밀 정보가 없다.
    DownloadFailed(String),
}

#[derive(Clone)]
pub struct ModelStore {
    root: PathBuf,
    manifest: Arc<Manifest>,
    space: Arc<dyn SpaceProbe>,
}

impl ModelStore {
    /// `root` 는 보통 `MAMData/models`.
    pub fn new(root: impl Into<PathBuf>, manifest: Manifest) -> Self {
        Self {
            root: root.into(),
            manifest: Arc::new(manifest),
            space: Arc::new(SystemSpace),
        }
    }

    /// 여유 공간 조회를 바꾼다 (시험에서 디스크 부족 모사).
    pub fn with_space_probe(mut self, space: Arc<dyn SpaceProbe>) -> Self {
        self.space = space;
        self
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn paths_of(&self, spec: &ModelSpec) -> ModelPaths {
        let dir = self.root.join(&spec.id);
        let files = spec
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    dir.join(f.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                )
            })
            .collect();
        ModelPaths {
            model_id: spec.id.clone(),
            dir,
            files,
        }
    }

    /// 다운로드 없이 상태만 본다 (파일 존재 + 크기).
    pub fn status(&self, model_id: &str) -> Result<ModelStatus> {
        let spec = self.manifest.get(model_id)?;
        let paths = self.paths_of(spec);
        let have = spec
            .files
            .iter()
            .zip(&paths.files)
            .filter(|(f, (_, p))| {
                std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() == f.size)
            })
            .count();
        Ok(match have {
            0 => ModelStatus::NotDownloaded,
            n if n == spec.files.len() => ModelStatus::Ready(paths),
            n => ModelStatus::Partial {
                have: n,
                total: spec.files.len(),
            },
        })
    }

    /// 없는 파일을 내려받아 모델을 준비한다. 이미 있으면 네트워크를 쓰지 않는다.
    /// `progress(파일 경로, 받은 바이트, 전체 바이트)`.
    pub fn ensure(
        &self,
        model_id: &str,
        progress: &mut dyn FnMut(&str, u64, u64),
    ) -> Result<ModelPaths> {
        if let ModelStatus::Ready(p) = self.status(model_id)? {
            return Ok(p);
        }
        let spec = self.manifest.get(model_id)?;
        let paths = self.paths_of(spec);
        for (f, (_, dest)) in spec.files.iter().zip(&paths.files) {
            if std::fs::metadata(dest).is_ok_and(|m| m.len() == f.size) {
                continue;
            }
            let name = f.path.clone();
            download_file(f, dest, &*self.space, &mut |r, t| progress(&name, r, t))?;
        }
        Ok(paths)
    }

    /// [`ensure`](Self::ensure) 결과를 기능 상태로 바꾼다. 실패해도 오류를 전파하지 않고 비활성 상태로 돌려준다.
    pub fn prepare(
        &self,
        model_id: &str,
        progress: &mut dyn FnMut(&str, u64, u64),
    ) -> FeatureState {
        match self.ensure(model_id, progress) {
            Ok(p) => FeatureState::Ready(p),
            Err(AiError::Offline { .. }) => FeatureState::Disabled(DisabledReason::Offline),
            Err(AiError::InsufficientSpace {
                needed, available, ..
            }) => FeatureState::Disabled(DisabledReason::DiskFull { needed, available }),
            Err(e) => FeatureState::Disabled(DisabledReason::DownloadFailed(e.to_string())),
        }
    }

    /// 저장된 파일의 SHA-256 을 모두 다시 계산해 매니페스트와 비교한다 (무결성 점검). 어긋난 상대 경로 목록을 돌려준다.
    pub fn verify(&self, model_id: &str) -> Result<Vec<String>> {
        let spec = self.manifest.get(model_id)?;
        let paths = self.paths_of(spec);
        let mut bad = Vec::new();
        for (f, (rel, p)) in spec.files.iter().zip(&paths.files) {
            match sha256_file(p) {
                Ok(h) if h == f.sha256 => {}
                _ => bad.push(rel.clone()),
            }
        }
        Ok(bad)
    }
}
