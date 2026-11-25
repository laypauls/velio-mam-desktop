//! 폴더 제거 밖의 연결 데이터 정리 두 가지:
//! - 스캔 대상 유형 변경의 "정리"(purge) — 제외한 유형의 연결 데이터 삭제 (폴더는 그대로 남는다)
//! - 기존 보관 데이터 1회 정리 — 폴더 제거가 위치만 detach 하던 때 남은 자산 (`purge::legacy` 참고)
//!
//! 둘 다 파괴적이라 시작 직전 백업을 만들고(`BackupFailed` 면 아무것도 바꾸지 않는다), 낮은 우선순위 500건 배치로 지운다.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use mam_db::Priority;
use mam_db::link_cleanup::{legacy_cleanup_done, mark_legacy_cleanup_done};

use super::removal::PURGE_BATCH;
use super::{Ingest, lock};
use crate::purge::{
    RangeSpec, count_in_range, legacy_candidates, purge_legacy_batch, purge_range_batch,
};
use crate::scan::{IngestError, load_folder};

/// [`Ingest::run_legacy_cleanup`] 결과
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegacyCleanupReport {
    /// 연결 데이터를 지운 자산 수
    pub purged: u64,
    /// 이미 끝난 정리라 아무것도 하지 않았다
    pub already_done: bool,
    /// 중단 요청으로 멈췄다 (완료 표시 없음 — 다음 실행이 이어서 한다)
    pub interrupted: bool,
}

impl Ingest {
    /// 스캔 대상 유형 변경의 정리 사전 점검: `folder_id` 폴더에 `excluded`(`media_assets.media_type` 값) 유형의 지울 위치가 있으면 먼저 백업한다.
    /// `BackupFailed` 이면 아무것도 바뀌지 않았다 — 그래서 유형 저장보다 먼저 부른다. 없는 폴더는 `NotFound`.
    pub(super) fn precheck_media_purge(
        &self,
        folder_id: i64,
        excluded: &[&str],
    ) -> Result<(), IngestError> {
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        let (volume_id, key) = (folder.volume_id, folder.path_key);
        let types: Vec<String> = excluded.iter().map(|s| (*s).to_owned()).collect();
        let n = self.env.pool.read(move |c| {
            let refs: Vec<&str> = types.iter().map(String::as_str).collect();
            let spec = RangeSpec {
                volume_id,
                root_key: &key,
                media_types: Some(&refs),
            };
            Ok(count_in_range(c, &spec))
        })??;
        if n > 0 {
            self.backup_before_purge()?;
        }
        Ok(())
    }

    /// 스캔 대상 유형 변경의 정리: `excluded` 유형 자산의 연결 데이터를 500건 배치로 지운다 (캐시 파일 포함, 다른 폴더에 활성 위치가
    /// 있는 자산은 이 폴더의 위치 행만). 지운 위치 행 수를 돌려준다.
    pub(super) fn purge_media_types(
        &self,
        folder_id: i64,
        excluded: &[&str],
    ) -> Result<u64, IngestError> {
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        let (volume_id, root_key) = (folder.volume_id, folder.path_key);
        let mut removed = 0u64;
        loop {
            let (clock, key) = (Arc::clone(&self.env.clock), root_key.clone());
            let types: Vec<String> = excluded.iter().map(|s| (*s).to_owned()).collect();
            let batch = self.env.write(Priority::Low, move |c| {
                let refs: Vec<&str> = types.iter().map(String::as_str).collect();
                let spec = RangeSpec {
                    volume_id,
                    root_key: &key,
                    media_types: Some(&refs),
                };
                Ok(purge_range_batch(c, clock.now_ms(), &spec, PURGE_BATCH)?)
            })?;
            if batch.locations == 0 {
                return Ok(removed);
            }
            self.remove_cache(&batch.purged);
            self.notify_embeddings(&batch.embedding_keys);
            removed += batch.locations as u64;
        }
    }

    /// 기존 보관 데이터 1회 정리를 실행한다. 이미 끝났으면 아무것도 하지 않는다 (`settings` 전용 키로 판정).
    /// 지울 후보가 있을 때만 첫 배치 직전에 백업하고, 백업이 실패하면 `BackupFailed` 로 멈춘다 (완료 표시 없음 — 다음 시작에 다시).
    /// 끝까지 돌면 완료 표시를 남긴다. `stop` 이 참이 되면 배치 사이에서 멈춘다 (멱등 — 다음 실행이 처음부터 다시 훑는다).
    pub fn run_legacy_cleanup(
        &self,
        stop: &dyn Fn() -> bool,
    ) -> Result<LegacyCleanupReport, IngestError> {
        let mut report = LegacyCleanupReport::default();
        if self.env.pool.read(legacy_cleanup_done)? {
            report.already_done = true;
            return Ok(report);
        }
        let (mut cursor, mut backed_up) = (0i64, false);
        loop {
            if stop() {
                report.interrupted = true;
                return Ok(report);
            }
            let after = cursor;
            let ids = self
                .env
                .pool
                .read(move |c| Ok(legacy_candidates(c, after, PURGE_BATCH)))??;
            let Some(&last) = ids.last() else { break };
            cursor = last;
            if !backed_up {
                self.backup_before_purge()?;
                backed_up = true;
            }
            let clock = Arc::clone(&self.env.clock);
            let (purged, keys) = self.env.write(Priority::Low, move |c| {
                Ok(purge_legacy_batch(c, clock.now_ms(), &ids)?)
            })?;
            self.remove_cache(&purged);
            self.notify_embeddings(&keys);
            report.purged += purged.len() as u64;
        }
        self.env.write(Priority::Low, |c| {
            mark_legacy_cleanup_done(c)?;
            Ok(())
        })?;
        if report.purged > 0 {
            *lock(&self.totals) = None;
        }
        Ok(report)
    }

    /// 시작 때 정리를 백그라운드 스레드에서 한 번 돌린다 (이미 도는 중이면 무시). 실패는 로그만 — 다음 시작에 다시 시도한다.
    pub(super) fn spawn_legacy_cleanup(&self) {
        let Some(me) = self.me.upgrade() else { return };
        if me.removal.legacy_running.swap(true, Ordering::AcqRel) {
            return;
        }
        let worker = Arc::clone(&me);
        let spawned = std::thread::Builder::new()
            .name("mam-legacy-cleanup".into())
            .spawn(move || {
                let stop = || worker.stop.is_set();
                match worker.run_legacy_cleanup(&stop) {
                    Ok(r) if r.purged > 0 => {
                        tracing::info!(purged = r.purged, "legacy preserved data cleaned up");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(%e, "legacy data cleanup did not finish; retries at the next start"),
                }
                worker.removal.legacy_running.store(false, Ordering::Release);
            });
        match spawned {
            Ok(h) => lock(&me.removal.handles).push(h),
            Err(e) => {
                tracing::warn!(%e, "cannot start legacy cleanup thread");
                me.removal.legacy_running.store(false, Ordering::Release);
            }
        }
    }
}
