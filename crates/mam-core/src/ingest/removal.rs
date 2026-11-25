//! 관리 폴더 제거 — 폴더를 "제거 중"으로 바꿔 화면에서 바로 빼고, 연결 데이터를 백그라운드에서 나눠 지운다.
//!
//! **제거 중 표시 = `managed_folders.enabled = 0` + `hidden = 1`** (스키마 변경 없음). `enabled = 0` 은 이미 "등록 해제 중"의
//! 뜻으로 쓰인다: 목록·스캔·감시·이전·점검이 건너뛰고, `hidden = 1` 은 검색·사이드바 개수에서 즉시 빠지게 한다.
//! 폴더 행은 **마지막에** 지우므로 도중에 꺼져도 `enabled = 0` 인 행이 남아 다음 실행이 이어서 끝낸다 (별도 상태 저장 불필요).
//! 삭제는 작업 대기열이 아니라 전용 스레드에서 한다 (`jobs.kind` CHECK 를 바꾸는 마이그레이션을 피하려고).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use mam_db::Priority;
use rusqlite::TransactionBehavior;
use serde::Serialize;

use super::{Ingest, lock};
use crate::purge::{
    EmbeddingKey, FolderRemovalImpact, RangeSpec, count_in_range, folder_removal_impact, purge_range_batch,
    remove_cache_files,
};
use crate::scan::{IngestError, detach_folder, load_all, load_folder};

/// 한 트랜잭션에서 지우는 위치 수
pub(super) const PURGE_BATCH: usize = 500;
/// 이벤트 `folders://removal-progress` 이름
pub const REMOVAL_PROGRESS_EVENT: &str = "folders://removal-progress";

/// 이벤트 페이로드 (계약 `FolderRemovalProgress`). `finished` 이면 끝난 것이고 `error` 가 있으면 실패 (다음 실행이 이어서 한다).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderRemovalProgress {
    pub folder_id: i64,
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub error: Option<String>,
}

/// 진행 알림 콜백 (Tauri 이벤트 발행용)
pub type RemovalListener = Arc<dyn Fn(FolderRemovalProgress) + Send + Sync>;
/// 파괴적 작업 직전 DB 백업. 실패하면 `Err` — 호출자는 아무것도 바꾸지 않는다.
pub type BackupHook = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// 임베딩 행을 지운 뒤 부른다 - 벡터 활성 맵 캐시가 지운 슬롯을 바로 버리게 한다 (`EmbedStore::notify_changed`).
pub type EmbeddingsChangedHook = Arc<dyn Fn(&[mam_vector::ModelKey]) + Send + Sync>;

/// [`Ingest::run_folder_removal`] 결과
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalRun {
    Finished,
    /// 종료 신호·중단 요청으로 멈췄다 (다음 실행이 이어서 한다)
    Interrupted,
}

#[derive(Default)]
pub(super) struct Removals {
    backup: std::sync::Mutex<Option<BackupHook>>,
    listener: std::sync::Mutex<Option<RemovalListener>>,
    cache_root: std::sync::Mutex<Option<PathBuf>>,
    embeddings_hook: std::sync::Mutex<Option<EmbeddingsChangedHook>>,
    running: std::sync::Mutex<HashSet<i64>>,
    pub(super) handles: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
    pub(super) legacy_running: std::sync::atomic::AtomicBool,
}

impl Ingest {
    /// 파괴적 작업 직전 백업을 만드는 콜백 (`mam-pre-remove-*`). 없으면 백업 없이 진행한다 — 앱은 항상 연결한다.
    pub fn set_backup_hook(&self, hook: BackupHook) {
        *lock(&self.removal.backup) = Some(hook);
    }

    /// 제거 진행 알림 콜백. [`Ingest::start`] 전에 호출한다.
    pub fn set_removal_listener(&self, listener: RemovalListener) {
        *lock(&self.removal.listener) = Some(listener);
    }

    /// 썸네일·미리보기 캐시 루트 (`MAMData/cache`). 정하지 않으면 캐시 파일은 지우지 않는다.
    pub fn set_cache_root(&self, root: PathBuf) {
        *lock(&self.removal.cache_root) = Some(root);
    }

    /// 임베딩을 지운 뒤 부를 콜백. 없으면 알리지 않는다.
    pub fn set_embeddings_changed_hook(&self, hook: EmbeddingsChangedHook) {
        *lock(&self.removal.embeddings_hook) = Some(hook);
    }

    /// 지운 임베딩의 모델 키를 콜백에 알린다 (키가 없으면 부르지 않는다).
    pub(super) fn notify_embeddings(&self, keys: &[EmbeddingKey]) {
        let hook = lock(&self.removal.embeddings_hook).clone();
        let Some(hook) = hook.filter(|_| !keys.is_empty()) else {
            return;
        };
        let parsed: Vec<mam_vector::ModelKey> = keys
            .iter()
            .filter_map(|k| {
                let kind = match k.kind.as_str() {
                    "visual" => mam_vector::VectorKind::Visual,
                    "semantic" => mam_vector::VectorKind::Semantic,
                    _ => return None,
                };
                Some(mam_vector::ModelKey {
                    kind,
                    model_id: k.model_id.clone(),
                    model_version: k.model_version.clone(),
                    dimension: usize::try_from(k.dimension).ok()?,
                })
            })
            .collect();
        hook(&parsed);
    }

    pub(super) fn backup_before_purge(&self) -> Result<(), IngestError> {
        let hook = lock(&self.removal.backup).clone();
        match hook {
            Some(f) => f().map_err(|e| {
                tracing::warn!(error = %e, "backup before purge failed; nothing was changed");
                IngestError::BackupFailed
            }),
            None => Ok(()),
        }
    }

    pub(super) fn remove_cache(&self, asset_ids: &[i64]) {
        if asset_ids.is_empty() {
            return;
        }
        let root = lock(&self.removal.cache_root).clone();
        if let Some(root) = root {
            remove_cache_files(&root, asset_ids);
        }
    }

    fn emit_removal(&self, p: FolderRemovalProgress) {
        let listener = lock(&self.removal.listener).clone();
        if let Some(f) = listener {
            f(p);
        }
    }

    /// 제거 확인 Dialog 용: 지워질 항목 수. 없는 폴더·제거 중인 폴더는 `NotFound`.
    pub fn folder_removal_impact(&self, folder_id: i64) -> Result<FolderRemovalImpact, IngestError> {
        self.env.pool.read(|c| {
            Ok((|| {
                let f = load_folder(c, folder_id)?;
                if !f.enabled {
                    return Err(IngestError::NotFound);
                }
                Ok(folder_removal_impact(c, f.volume_id, &f.path_key)?)
            })())
        })?
    }

    /// 제거를 시작한다: 사전 DB 백업 -> 제거 중 표시(목록·검색에서 즉시 빠짐) -> 감시·대기 작업 중지. 삭제 스레드는 시작하지 않는다.
    /// 이미 제거 중이면 아무것도 바꾸지 않고 `false`. 백업 실패는 `BackupFailed` (아무것도 바뀌지 않음).
    pub fn begin_folder_removal(&self, folder_id: i64) -> Result<bool, IngestError> {
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        if !folder.enabled {
            return Ok(false);
        }
        self.backup_before_purge()?;
        self.watchers.stop(folder_id);
        let marked = self.env.write(Priority::High, move |c| {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // 스캔 배치는 `enabled = 0` 이면 아무것도 쓰지 않는다 (제거 후 위치가 되살아나지 않게)
            let n = tx.execute(
                "UPDATE managed_folders SET enabled = 0, hidden = 1 WHERE folder_id = ?1 AND enabled = 1",
                [folder_id],
            )?;
            if n == 0 {
                return Ok(false);
            }
            tx.execute(
                "DELETE FROM jobs WHERE folder_id = ?1 AND status IN ('pending', 'failed')",
                [folder_id],
            )?;
            tx.commit()?;
            Ok(true)
        })?;
        *lock(&self.totals) = None;
        lock(&self.watch_failed).remove(&folder_id);
        self.jobs.notify();
        Ok(marked)
    }

    /// 관리 폴더를 제거한다. 원본 파일은 건드리지 않는다. 반환 시점에 폴더는 목록·검색에서 빠져 있고,
    /// 연결 데이터 삭제는 백그라운드에서 이어진다 (`folders://removal-progress`). 다시 호출해도 안전하다.
    pub fn remove_folder(&self, folder_id: i64) -> Result<(), IngestError> {
        self.begin_folder_removal(folder_id)?;
        self.spawn_removal(folder_id);
        Ok(())
    }

    /// 제거 중인(`enabled = 0`) 폴더마다 삭제를 이어서 시작한다 (앱 시작 때). 시작한 수를 돌려준다.
    pub fn resume_removals(&self) -> Result<usize, IngestError> {
        let ids: Vec<i64> = self
            .env
            .pool
            .read(|c| Ok(load_all(c)))??
            .into_iter()
            .filter(|f| !f.enabled)
            .map(|f| f.folder_id)
            .collect();
        for &id in &ids {
            // 예전 버전이 `enabled = 0` 만 남기고 멈춘 경우에도 검색에서 빠지게 한다
            self.env.write(Priority::High, move |c| {
                c.execute(
                    "UPDATE managed_folders SET hidden = 1 WHERE folder_id = ?1",
                    [id],
                )?;
                Ok(())
            })?;
            self.spawn_removal(id);
        }
        Ok(ids.len())
    }

    fn spawn_removal(&self, folder_id: i64) {
        let Some(me) = self.me.upgrade() else { return };
        if !lock(&self.removal.running).insert(folder_id) {
            return; // 이미 도는 중
        }
        let worker = Arc::clone(&me);
        let spawned = std::thread::Builder::new()
            .name(format!("mam-removal-{folder_id}"))
            .spawn(move || {
                let stop = || worker.stop.is_set();
                if let Err(e) = worker.run_folder_removal(folder_id, &stop) {
                    tracing::warn!(%e, folder_id, "folder removal failed; it resumes at the next start");
                    worker.emit_removal(FolderRemovalProgress {
                        folder_id,
                        done: 0,
                        total: 0,
                        finished: true,
                        error: Some("removal_failed".into()),
                    });
                }
                lock(&worker.removal.running).remove(&folder_id);
            });
        match spawned {
            Ok(h) => {
                let mut hs = lock(&self.removal.handles);
                hs.retain(|h| !h.is_finished());
                hs.push(h);
            }
            Err(e) => {
                tracing::warn!(%e, folder_id, "cannot start removal thread");
                lock(&self.removal.running).remove(&folder_id);
            }
        }
    }

    /// 제거·정리 스레드가 모두 끝날 때까지 기다린다 (시험·진단용).
    pub fn wait_removals(&self) {
        let handles: Vec<_> = std::mem::take(&mut *lock(&self.removal.handles));
        for h in handles {
            let _ = h.join();
        }
    }

    /// 제거 중인 폴더의 삭제를 끝까지 (또는 `stop` 이 참이 될 때까지) 실행한다. 배치(500)마다 `stop` 을 확인한다.
    /// 위치 행은 연결 데이터와 같은 트랜잭션에서 지우므로 처음부터 다시 불러도 이어서 한다.
    /// 폴더 행이 이미 없으면 끝난 것으로 본다. 제거 중이 아닌(`enabled = 1`) 폴더는 `InvalidInput`.
    pub fn run_folder_removal(
        &self,
        folder_id: i64,
        stop: &dyn Fn() -> bool,
    ) -> Result<RemovalRun, IngestError> {
        let folder = match self.env.pool.read(|c| Ok(load_folder(c, folder_id)))? {
            Ok(f) => f,
            Err(IngestError::NotFound) => return Ok(RemovalRun::Finished),
            Err(e) => return Err(e),
        };
        if folder.enabled {
            return Err(IngestError::InvalidInput("folder is not being removed"));
        }
        let (volume_id, root_key) = (folder.volume_id, folder.path_key);
        let total = {
            let key = root_key.clone();
            self.env.pool.read(move |c| {
                let spec = RangeSpec {
                    volume_id,
                    root_key: &key,
                    media_types: None,
                };
                Ok(count_in_range(c, &spec))
            })??
        };
        let total = u64::try_from(total).unwrap_or(0);
        let progress = |done, finished| FolderRemovalProgress {
            folder_id,
            done,
            total,
            finished,
            error: None,
        };
        let mut done = 0u64;
        self.emit_removal(progress(0, false));
        loop {
            if stop() {
                return Ok(RemovalRun::Interrupted);
            }
            let (clock, key) = (Arc::clone(&self.env.clock), root_key.clone());
            let batch = self.env.write(Priority::Low, move |c| {
                let spec = RangeSpec {
                    volume_id,
                    root_key: &key,
                    media_types: None,
                };
                Ok(purge_range_batch(c, clock.now_ms(), &spec, PURGE_BATCH)?)
            })?;
            if batch.locations == 0 {
                break;
            }
            self.remove_cache(&batch.purged);
            self.notify_embeddings(&batch.embedding_keys);
            done += batch.locations as u64;
            self.emit_removal(progress(done, false));
        }
        self.env
            .write(Priority::High, move |c| detach_folder(c, folder_id))?;
        *lock(&self.totals) = None;
        self.jobs.notify();
        self.emit_removal(progress(done, true));
        Ok(RemovalRun::Finished)
    }
}
