//! 스캔·감시가 쓰는 "제한 시간 파일 호출" — `io_guard` 위의 얇은 어댑터.
//!
//! 막힐 수 있는 볼륨(네트워크·이동식)에서만 작업 스레드를 거친다. 내장 디스크(`Fixed`)는 호출을 그대로 한다 —
//! 파일 100만 개마다 스레드를 만드는 비용을 내장 디스크에는 물리지 않는다.
//! 결과는 스레드 사이에서 나눠 쓰므로 오류는 복제 가능한 [`Fail`] 로 바꿔 돌려준다.

use std::io;
use std::path::Path;
use std::sync::Arc;

use super::errors::{core_code, io_code};
use super::folders::FolderRow;
use super::io_guard::Guarded;
use super::walk::{Listing, WalkRules, classify_file, is_hidden_or_system_dir, list_dir};
use super::work::ScanEnv;
use crate::identity::FileMeta;
use crate::reconcile::MediaType;
use crate::{CoreError, VolumeKind};

/// 파일 호출의 실패 — 오류 종류 코드(`scan_errors.error_code`)와 "없음" 여부만 남긴 복제 가능한 형태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub code: &'static str,
    pub not_found: bool,
    pub message: String,
}

impl Fail {
    pub fn io(e: &io::Error) -> Self {
        Self {
            code: io_code(e),
            not_found: e.kind() == io::ErrorKind::NotFound,
            message: e.to_string(),
        }
    }

    pub fn core(e: &CoreError) -> Self {
        Self {
            code: core_code(e),
            not_found: matches!(e, CoreError::Io { source, .. } if source.kind() == io::ErrorKind::NotFound),
            message: e.to_string(),
        }
    }
}

/// `scan_errors.error_code` — 제한 시간을 넘겨 이번에는 읽지 못했다 (missing 이 아니다)
pub const UNRESPONSIVE_CODE: &str = "unresponsive";
/// 위 코드에 붙는 메시지 (경로·계정 같은 값은 싣지 않는다)
pub const UNRESPONSIVE_MESSAGE: &str = "no response from the volume within the time limit";

/// `symlink_metadata` 결과 중 반영 경로가 쓰는 부분
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

impl ScanEnv {
    /// 블로킹 호출 `f` 를 이 볼륨 종류에 맞게 부른다 — 막힐 수 있는 볼륨이면 제한 시간을 건다.
    pub fn guarded_io<T, F>(&self, kind: VolumeKind, volume_id: i64, key: String, f: F) -> Guarded<T>
    where
        T: Clone + Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        match kind {
            VolumeKind::Fixed => Guarded::Done(f()),
            VolumeKind::Removable | VolumeKind::Network => self.io.run(&key, volume_id, f),
        }
    }

    /// 디렉터리 1개의 목록화
    pub(crate) fn list_dir_guarded(
        &self,
        folder: &FolderRow,
        dir: &Path,
        rules: &WalkRules,
    ) -> Guarded<Result<Listing, Fail>> {
        let (dir, rules) = (dir.to_path_buf(), rules.clone());
        let key = format!("ls:{}", dir.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            list_dir(&dir, &rules).map_err(|e| Fail::io(&e))
        })
    }

    /// 파일 1개의 식별 정보(`identity.stat`)
    pub(crate) fn stat_guarded(&self, folder: &FolderRow, path: &Path) -> Guarded<Result<FileMeta, Fail>> {
        let (path, identity) = (path.to_path_buf(), Arc::clone(&self.identity));
        let key = format!("st:{}", path.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            identity.stat(&path).map_err(|e| Fail::core(&e))
        })
    }

    /// 경로의 종류 (심볼릭 링크를 따라가지 않는다). 없으면 `Fail { not_found }`
    pub(crate) fn entry_kind_guarded(
        &self,
        folder: &FolderRow,
        path: &Path,
    ) -> Guarded<Result<EntryKind, Fail>> {
        let path = path.to_path_buf();
        let key = format!("sm:{}", path.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            std::fs::symlink_metadata(&path)
                .map(|m| {
                    let t = m.file_type();
                    if t.is_symlink() {
                        EntryKind::Symlink
                    } else if t.is_file() {
                        EntryKind::File
                    } else if t.is_dir() {
                        EntryKind::Dir
                    } else {
                        EntryKind::Other
                    }
                })
                .map_err(|e| Fail::io(&e))
        })
    }

    /// 루트 아래 경로(`names`)의 조상 중 숨김·시스템 속성 디렉터리가 있는가 — 한 번의 제한 시간 호출로 조상 전부를 본다.
    /// 루트 자신은 검사하지 않는다 (볼륨 루트는 숨김+시스템 속성일 수 있다).
    pub(crate) fn hidden_ancestor_guarded(&self, folder: &FolderRow, names: &[String]) -> Guarded<bool> {
        let (root, names) = (folder.root_path(), names.to_vec());
        let key = format!("hs:{}", root.join(names.join("/")).display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            let mut p = root;
            names.iter().any(|n| {
                p.push(n);
                is_hidden_or_system_dir(&p)
            })
        })
    }

    /// 폴더 루트에 닿는가 (`metadata`) — 없거나 읽을 수 없으면 `Err`
    pub(crate) fn root_reachable_guarded(&self, folder: &FolderRow) -> Guarded<Result<(), Fail>> {
        let root = folder.root_path();
        let key = format!("md:{}", root.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            std::fs::metadata(&root).map(|_| ()).map_err(|e| Fail::io(&e))
        })
    }

    /// 폴더 루트를 읽을 수 있는가 (`read_dir` 첫 호출)
    pub(crate) fn root_readable_guarded(&self, folder: &FolderRow) -> Guarded<Result<(), Fail>> {
        let root = folder.root_path();
        let key = format!("rd:{}", root.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            std::fs::read_dir(&root).map(|_| ()).map_err(|e| Fail::io(&e))
        })
    }

    /// 확장자가 없을 때 앞부분을 읽어 종류를 판별하는 호출
    pub(crate) fn classify_guarded(
        &self,
        folder: &FolderRow,
        path: &Path,
    ) -> Guarded<Result<Option<MediaType>, Fail>> {
        let path = path.to_path_buf();
        let key = format!("cl:{}", path.display());
        self.guarded_io(folder.volume_kind, folder.volume_id, key, move || {
            classify_file(&path).map_err(|e| Fail::io(&e))
        })
    }
}
