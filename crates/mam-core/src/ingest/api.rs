//! 폴더 관리 API: 등록·제거·재스캔·조회·오류 재시도.

use std::path::{Path, PathBuf};

use mam_db::Priority;

use super::Ingest;
use crate::jobs::JobKind;
use crate::registry::register_volume;
use crate::scan::{
    FolderNode, IngestError, ManagedFolderView, MediaTypeCounts, MediaTypeSet, ScanErrorView,
    add_folder_row, count_by_type, list_errors, list_folder_views, list_subfolders, load_folder,
    retry_targets, set_alias, set_hidden, set_media_types,
};
use crate::watch::WatchTarget;

/// 한 번에 가져올 수 있는 오류 목록 상한 (계약 `MAX_PAGE_LIMIT`)
pub const MAX_PAGE_LIMIT: usize = 200;

impl Ingest {
    /// 관리 폴더로 등록하고 첫 스캔을 예약한다. 같은 폴더는 `AlreadyRegistered`, 상위·하위 폴더와 겹치면 `NestedFolder`.
    pub fn add_folder(&self, path: &str) -> Result<ManagedFolderView, IngestError> {
        self.add_folder_with_types(path, MediaTypeSet::ALL)
    }

    /// [`Self::add_folder`] 에 스캔 대상 유형을 지정한다. 모두 꺼져 있으면 `InvalidInput`.
    pub fn add_folder_with_types(
        &self,
        path: &str,
        types: MediaTypeSet,
    ) -> Result<ManagedFolderView, IngestError> {
        if !types.any() {
            return Err(IngestError::InvalidInput("mediaTypes"));
        }
        let canonical = self.resolve_new_root(path)?;

        let info = self.volumes.volume_of(&canonical)?;
        // 꺼내기 준비 중인 디스크에는 폴더를 추가하지 않는다 (등록이 볼륨을 온라인으로 되돌리고 감시를 시작한다) - 먼저 감시를 다시 시작한다
        if self.key_held(&info.key) {
            return Err(IngestError::Offline);
        }
        let (rel_root, path_key) = crate::scan::root_keys(&info, &canonical)?;
        let display = crate::volume::strip_verbatim(&canonical.to_string_lossy());
        let clock = std::sync::Arc::clone(&self.env.clock);
        let folder_id = self.env.write(Priority::High, move |c| {
            let volume_id = register_volume(c, &*clock, &info)?;
            add_folder_row(
                c,
                clock.now_ms(),
                volume_id,
                &rel_root,
                &path_key,
                &display,
                types,
            )
        })?;
        self.enqueue_scan(folder_id, true)?;
        self.start_watch(folder_id);
        // 새 디스크 안내의 대기 목록에서 이 디스크를 뺀다
        self.refresh_pending();
        self.view_of(folder_id)
    }

    /// 폴더 등록·이전의 대상 경로 검증: 절대 경로·존재·디렉터리·읽기 권한·MAMData/백업과 겹침 없음. 정규화된 경로를 돌려준다.
    pub(super) fn resolve_new_root(&self, path: &str) -> Result<PathBuf, IngestError> {
        if path.is_empty() || path.contains('\0') || !Path::new(path).is_absolute() {
            return Err(IngestError::InvalidInput("path"));
        }
        let canonical = match std::fs::canonicalize(path) {
            Ok(p) => p,
            // 연결되지 않은 공유는 "없는 경로" 가 아니라 오프라인이다
            Err(_) if crate::volume::strip_verbatim(path).starts_with(r"\\") => {
                return Err(IngestError::Offline);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(IngestError::InvalidInput("path"));
            }
            Err(e) => return Err(e.into()),
        };
        if !canonical.is_dir() {
            return Err(IngestError::InvalidInput("path is not a directory"));
        }
        std::fs::read_dir(&canonical)?; // 읽기 권한 확인 (실패하면 Io)
        // MAMData·백업 폴더와 어느 방향으로든 겹치면 거부 — 하위면 스캔 제외 규칙에 막혀 조용히 비고,
        // 상위면 초기화·백업 정리가 원본 트리를 건드릴 수 있다
        for excluded in &self.config.auto_excludes {
            if crate::storage::paths_overlap(&canonical, excluded) {
                return Err(IngestError::OverlapsManagedData {
                    contains: !crate::storage::is_same_or_inside(&canonical, excluded),
                });
            }
        }
        Ok(canonical)
    }

    pub(super) fn view_of(&self, folder_id: i64) -> Result<ManagedFolderView, IngestError> {
        self.list_folders()?
            .into_iter()
            .find(|f| f.folder_id == folder_id)
            .ok_or(IngestError::NotFound)
    }

    pub fn list_folders(&self) -> Result<Vec<ManagedFolderView>, IngestError> {
        let statuses = self.jobs.folder_scan_statuses()?;
        let paused = self.jobs.is_paused(JobKind::Scan);
        let held = self.ejected_volumes();
        let ejectable = self.ejectable_volumes();
        let mut views = self
            .env
            .pool
            .read(|c| Ok(list_folder_views(c, &statuses, paused, &held, &ejectable)))??;
        self.fill_detection(&mut views)?;
        Ok(views)
    }

    /// 수동 재스캔 예약. 볼륨이 오프라인이면 `Offline`.
    pub fn rescan_folder(&self, folder_id: i64) -> Result<(), IngestError> {
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        if !folder.enabled {
            return Err(IngestError::Busy); // 제거 중 - 삭제가 끝나면 폴더 자체가 사라진다
        }
        if !folder.online {
            return Err(IngestError::Offline);
        }
        self.enqueue_scan(folder_id, true)?;
        Ok(())
    }

    /// 폴더 숨김 토글 — 보기 속성이라 스캔·감시·분석은 그대로 계속된다. 없는 폴더·등록 해제 중은 `NotFound`.
    pub fn set_folder_hidden(&self, folder_id: i64, hidden: bool) -> Result<(), IngestError> {
        self.env
            .write(Priority::High, move |c| set_hidden(c, folder_id, hidden))?;
        // 요약의 전체 자산 수는 숨김 상태를 반영하므로 캐시를 버린다
        *super::lock(&self.totals) = None;
        Ok(())
    }

    /// 폴더 별칭 설정/해제 — 표시 전용이라 색인·캐시에 영향 없다. 빈 값은 해제, 64자 초과는 `InvalidInput`.
    pub fn set_folder_alias(
        &self,
        folder_id: i64,
        alias: Option<String>,
    ) -> Result<(), IngestError> {
        self.env.write(Priority::High, move |c| {
            set_alias(c, folder_id, alias.as_deref())
        })?;
        Ok(())
    }

    /// 스캔 대상 유형을 바꾼다. 모두 꺼져 있으면 `InvalidInput`, 없는 폴더·등록 해제 중은 `NotFound`.
    /// `purge` 이고 제외된 유형이 있으면 그 유형 자산의 **연결 데이터(태그·메모·컬렉션 소속·AI 결과·썸네일 등)를 영구 삭제**한다
    /// (자산 행은 `deleted` 표시, 검색 인덱스에 `delete` 이벤트, 500건 배치·Low). 지울 게 있으면 먼저 DB 를 백업하고 실패하면
    /// `BackupFailed` 로 아무것도 바꾸지 않는다. 원본은 건드리지 않는다. 마지막에 재스캔을 예약해(오프라인이면 재연결 때)
    /// 새로 켠 유형을 수집한다. 지운 위치 수를 돌려준다.
    pub fn set_folder_media_types(
        &self,
        folder_id: i64,
        types: MediaTypeSet,
        purge: bool,
    ) -> Result<u64, IngestError> {
        let excluded = types.excluded();
        let purging = purge && !excluded.is_empty();
        if purging {
            self.precheck_media_purge(folder_id, &excluded)?;
        }
        let clock = std::sync::Arc::clone(&self.env.clock);
        self.env.write(Priority::High, move |c| {
            set_media_types(c, clock.now_ms(), folder_id, types)
        })?;
        let mut removed = 0u64;
        if purging {
            removed = self.purge_media_types(folder_id, &excluded)?;
            if removed > 0 {
                *super::lock(&self.totals) = None;
            }
        }
        match self.rescan_folder(folder_id) {
            // 오프라인이면 재연결 때 재스캔된다
            Ok(()) | Err(IngestError::Offline) => {}
            Err(e) => return Err(e),
        }
        Ok(removed)
    }

    /// 폴더 하위 활성 위치의 유형별 개수 (대상 유형 변경 전 영향 안내용). 없는 폴더는 `NotFound`.
    pub fn count_folder_assets_by_type(
        &self,
        folder_id: i64,
    ) -> Result<MediaTypeCounts, IngestError> {
        self.env.pool.read(|c| {
            Ok((|| {
                let folder = load_folder(c, folder_id)?;
                if !folder.enabled {
                    return Err(IngestError::NotFound);
                }
                count_by_type(c, folder.volume_id, &folder.path_key)
            })())
        })?
    }

    pub fn list_subfolders(
        &self,
        folder_id: i64,
        rel_path: Option<&str>,
    ) -> Result<Vec<FolderNode>, IngestError> {
        self.env.pool.read(|c| {
            Ok((|| {
                let folder = load_folder(c, folder_id)?;
                list_subfolders(c, &folder, rel_path)
            })())
        })?
    }

    /// 미해결 스캔 오류 (최신 순). `cursor` 는 이전 페이지의 `next_cursor`.
    pub fn list_scan_errors(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ScanErrorView>, i64, Option<String>), IngestError> {
        let after = match cursor {
            None => None,
            Some(s) => Some(
                s.parse::<i64>()
                    .map_err(|_| IngestError::InvalidInput("cursor"))?,
            ),
        };
        let limit = limit.clamp(1, MAX_PAGE_LIMIT);
        let (items, total, next) = self.env.pool.read(|c| Ok(list_errors(c, after, limit)))??;
        Ok((items, total, next.map(|n| n.to_string())))
    }

    /// 선택한(`None` = 전체) 오류가 있는 폴더를 다시 스캔한다 — 같은 경로가 이번에는 읽히면 오류는 자동 해결된다.
    /// 재스캔을 예약한 오류 수를 돌려준다 (오프라인·제거된 폴더의 오류는 제외).
    pub fn retry_scan_errors(&self, ids: Option<&[i64]>) -> Result<usize, IngestError> {
        let targets = self.env.pool.read(|c| Ok(retry_targets(c, ids)))??;
        let mut folders: Vec<i64> = targets.iter().map(|(_, f)| *f).collect();
        folders.sort_unstable();
        folders.dedup();
        let mut scheduled = 0;
        for folder_id in folders {
            match self.rescan_folder(folder_id) {
                Ok(()) => scheduled += targets.iter().filter(|(_, f)| *f == folder_id).count(),
                Err(IngestError::Offline | IngestError::NotFound | IngestError::Busy) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(scheduled)
    }

    pub(super) fn start_watch(&self, folder_id: i64) {
        let _ctl = super::lock(&self.watch_ctl);
        self.start_watch_locked(folder_id);
    }

    /// `watch_ctl` 을 쥔 채로 호출한다.
    pub(super) fn start_watch_locked(&self, folder_id: i64) {
        let f = match self.env.pool.read(|c| Ok(load_folder(c, folder_id))) {
            Ok(Ok(f)) if f.enabled && f.online && !self.volume_held(f.volume_id) => f,
            _ => return,
        };
        let out = self.watchers.start(
            &WatchTarget::of(&f),
            f.watch_mode.as_deref(),
            f.watch_options.as_deref(),
        );
        if let Some(e) = out.last_error {
            // NAS 등 감시가 안 되는 곳 — 주기 재스캔이 대신한다. 같은 폴더의 반복 경고는 한 번만
            if super::lock(&self.watch_failed).insert(folder_id) {
                tracing::warn!(%e, folder_id, "cannot watch folder; periodic rescan only");
            }
        }
    }
}
