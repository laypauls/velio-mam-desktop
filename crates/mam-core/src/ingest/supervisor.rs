//! 주기 점검 (D6): 볼륨 연결 감지 -> 온라인 전환 + 재스캔 예약, 마지막 스캔이 오래된 폴더 재스캔 예약, 감시 재시도.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use mam_db::{Priority, settings};

use super::Ingest;
use super::rescan_policy::SlotCtx;
use crate::VolumeKind;
use crate::processing::{BackfillReport, backfill_jobs};
use crate::registry::{set_volume_online, sync_volumes_holding};
use crate::scan::Guarded;
use crate::scan::{IngestError, load_all};

/// 한 번의 점검 결과 (시험·로그용).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SuperviseReport {
    pub went_online: Vec<i64>,
    pub went_offline: Vec<i64>,
    /// 재스캔을 예약한 폴더
    pub scheduled: Vec<i64>,
    /// 처리 작업을 다시 등록했다 (첫 점검과 볼륨 연결 때, P2-3)
    pub backfill: Option<BackfillReport>,
}

impl Ingest {
    /// 점검 1회. 스레드가 주기적으로 부르며, 시험에서는 직접 부른다.
    pub fn supervise_once(&self) -> Result<SuperviseReport, IngestError> {
        let mut report = SuperviseReport::default();
        let present = self.sync_volume_states(&mut report)?;
        // 새 디스크 판정 (실패해도 연결 상태 점검은 계속한다)
        if let Err(e) = self.detect_new_volumes(&present) {
            tracing::warn!(%e, "cannot detect new volumes");
        }

        // 볼륨이 돌아온 폴더: 감시 재개 + 놓친 변경을 따라잡는 재스캔
        let folders = self.env.pool.read(|c| Ok(load_all(c)))??;
        for f in &folders {
            if report.went_offline.contains(&f.volume_id) {
                self.watchers().stop(f.folder_id);
            }
            if report.went_online.contains(&f.volume_id) && f.enabled {
                self.watchers().stop(f.folder_id); // 마운트 지점이 바뀌었을 수 있다 — 새 경로로 다시 건다
                self.start_watch(f.folder_id);
                if self.enqueue_scan(f.folder_id, false)? {
                    report.scheduled.push(f.folder_id);
                }
            }
        }

        // 주기 재스캔: 폴더별 정책(없으면 전역 간격)이 시작하라고 하거나 첫 스캔이 끝나지 않은 폴더 (이미 대기·실행·실패 중이면 건너뜀)
        let (s, _) = self.env.pool.read(settings::load)?;
        // 설정 변경(스캔 동시성 상한)을 점검 주기 안에 반영한다
        self.jobs.set_scan_cap(s.max_concurrent_scans as usize);
        let now = self.env.clock.now_ms();
        let zone = &*self.config.local_zone;
        let busy: HashSet<i64> = self
            .jobs
            .folder_scan_statuses()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let first_pass = !self.on_start_pass.swap(true, Ordering::Relaxed);
        for f in folders.iter().filter(|f| f.enabled && f.online) {
            if !self.watchers().is_settled(f.folder_id) {
                self.start_watch(f.folder_id);
            }
            let policy = self.effective_policy_of(f, s.rescan_interval_hours);
            let cx = SlotCtx {
                now_ms: now,
                zone,
                last_full_scan_at: f.last_full_scan_at,
            };
            if policy.is_due(&cx, first_pass) && !busy.contains(&f.folder_id) {
                // 한 폴더의 예약 실패가 나머지 폴더의 점검(특히 `on_start` 첫 점검)을 막지 않게 기록만 하고 계속한다
                match self.enqueue_scan(f.folder_id, false) {
                    Ok(true) => report.scheduled.push(f.folder_id),
                    Ok(false) => {}
                    Err(e) => tracing::warn!(%e, folder_id = f.folder_id, "cannot schedule rescan"),
                }
            }
        }

        // 스캔 멈춤 감시 (3분 넘게 같은 경로) — 폴더 상태는 `list_folders` 가 같은 값을 계산해 싣는다
        self.check_stalls();

        // 처리 작업 보충: 처리 기능을 쓰기 전에 등록된 자산, 오프라인이라 건너뛴 자산(볼륨이 돌아왔을 때), 사라진 작업
        if self.env.schedule_processing.load(Ordering::Relaxed)
            && (!self.backfilled.swap(true, Ordering::Relaxed) || !report.went_online.is_empty())
        {
            let clock = Arc::clone(&self.env.clock);
            let r = backfill_jobs(&self.env.pool, move || clock.now_ms())
                .map_err(|e| IngestError::Core(crate::CoreError::Internal(e.to_string())))?;
            // 장면 검출: pending 인데 작업이 없는 영상 재등록 (오프라인이라 끝낸 작업·사라진 작업)
            let now = self.env.clock.now_ms();
            crate::scene::requeue_pending(&self.env.pool, now)
                .map_err(|e| IngestError::Core(crate::CoreError::Internal(e.to_string())))?;
            self.jobs.notify();
            report.backfill = Some(r);
        }
        Ok(report)
    }

    /// 저장된 설정의 스캔 동시성 상한을 대기열에 반영한다 (읽지 못하면 기본값 그대로).
    pub(super) fn apply_scan_cap_setting(&self) {
        match self.env.pool.read(settings::load) {
            Ok((s, _)) => self.jobs.set_scan_cap(s.max_concurrent_scans as usize),
            Err(e) => tracing::warn!(%e, "cannot read the scan concurrency setting"),
        }
    }

    /// OS 볼륨 목록(로컬)과 공유 접근성(네트워크)으로 `volumes.availability` 를 맞춘다.
    fn sync_volume_states(
        &self,
        report: &mut SuperviseReport,
    ) -> Result<Vec<crate::VolumeInfo>, IngestError> {
        let clock = Arc::clone(&self.env.clock);
        let provider = Arc::clone(&self.volumes);
        let held = self.eject.shared();
        // 꺼내기 준비 볼륨 집합은 DB 쓰기 스레드 안에서 읽는다 - 준비의 (붙들기 -> 오프라인 쓰기) 순서와 어긋나 온라인으로 되돌리지 않게
        let local = self.env.write(Priority::Low, move |c| {
            let held = super::lock(&held).volumes.keys().copied().collect();
            Ok(sync_volumes_holding(c, &*clock, &*provider, &held)?)
        })?;
        // 붙들린 디스크가 실제로 빠졌다 - 붙들기를 푼다 (다시 꽂으면 일반 재연결 처리)
        for id in &local.released {
            if self.release_held(*id) {
                tracing::info!(
                    volume_id = *id,
                    "ejected volume is gone; eject hold released"
                );
            }
        }
        report.went_online.extend(local.went_online);
        report.went_offline.extend(local.went_offline);
        let present = local.present;

        // 네트워크 볼륨은 열거되지 않는다 — 마운트 지점에 접근해 본다 (응답 없는 공유에서 오래 걸릴 수 있어 쓰기 스레드 밖에서)
        let nets: Vec<(i64, Option<String>, bool)> = self.env.pool.read(|c| {
            let mut stmt = c.prepare(
                "SELECT volume_id, last_mount_point, availability = 'online' FROM volumes
                 WHERE kind = 'network' AND volume_id IN (SELECT volume_id FROM managed_folders)",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })?;
        for (volume_id, mount, was_online) in nets {
            // 제한 시간 안에 응답하지 않으면 닿지 않는 것으로 본다 (오프라인 — 삭제가 아니다, 재연결 때 재스캔)
            let reachable = mount.as_deref().is_some_and(|m| {
                let target = m.to_owned();
                matches!(
                    self.env.guarded_io(
                        VolumeKind::Network,
                        volume_id,
                        format!("net:{m}"),
                        move || std::fs::metadata(&target).is_ok(),
                    ),
                    Guarded::Done(true)
                )
            });
            if reachable == was_online {
                continue;
            }
            let clock = Arc::clone(&self.env.clock);
            self.env.write(Priority::Low, move |c| {
                Ok(set_volume_online(c, &*clock, volume_id, reachable)?)
            })?;
            if reachable {
                report.went_online.push(volume_id);
            } else {
                report.went_offline.push(volume_id);
            }
        }
        Ok(present)
    }
}
