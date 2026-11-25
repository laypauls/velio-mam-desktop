//! 스캔 차선 (결정 E) — 같은 볼륨의 스캔은 한 번에 하나, 볼륨이 다르면 동시에 진행, 전체 동시 수는 상한(설정)까지.
//!
//! 같은 HDD 에서 스캔 2개를 돌리면 오히려 느려지고, NAS 하나가 멈춰도 다른 볼륨의 스캔은 계속돼야 한다.
//! 차선 키는 폴더의 `volume_id` (폴더 행을 찾을 수 없는 작업은 폴더마다 따로 쓰도록 `-folder_id`).
//! 다른 종류 작업의 동시성(종류별 상한)과 스캔의 일시정지·취소·우선순위·재개는 그대로다.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, Transaction, params};

use super::{JobQueue, lock};
use crate::jobs::{Job, JobKind, SCAN_CAP_RANGE};

/// 실행 중인 스캔 작업이 차지한 차선 (작업 id -> 차선)
#[derive(Default)]
pub(super) struct Lanes {
    busy: Mutex<HashMap<i64, i64>>,
}

impl JobQueue {
    /// 종류별 현재 동시성 상한 (스캔은 설정으로 바뀐다)
    pub(super) fn cap_of(&self, kind: JobKind) -> usize {
        if kind == JobKind::Scan {
            self.scan_cap.load(Ordering::SeqCst)
        } else {
            self.caps[kind.idx()]
        }
    }

    /// 동시에 도는 스캔의 전체 상한
    pub fn scan_cap(&self) -> usize {
        self.scan_cap.load(Ordering::SeqCst)
    }

    /// 스캔 전체 상한을 바꾼다 (허용 범위로 맞춘다). 이미 도는 스캔은 끊지 않고, 줄이면 자연히 끝날 때 줄어든다.
    pub fn set_scan_cap(&self, cap: usize) {
        let cap = cap.clamp(*SCAN_CAP_RANGE.start(), *SCAN_CAP_RANGE.end());
        if self.scan_cap.swap(cap, Ordering::SeqCst) != cap {
            self.notify();
        }
    }

    pub(super) fn busy_lanes(&self) -> Vec<i64> {
        lock(&self.lanes.busy).values().copied().collect()
    }

    pub(super) fn lane_occupy(&self, job_id: i64, lane: i64) {
        lock(&self.lanes.busy).insert(job_id, lane);
    }

    pub(super) fn lane_release(&self, job_id: i64) {
        lock(&self.lanes.busy).remove(&job_id);
    }
}

/// 비어 있는 차선의 가장 앞선 대기 스캔 작업 1건과 그 차선 (`busy` = 지금 스캔이 도는 차선들).
pub(super) fn next_scan_job(
    tx: &Transaction<'_>,
    now: i64,
    max_priority: i64,
    busy: &[i64],
) -> rusqlite::Result<Option<(Job, i64)>> {
    // 정수만 이어 붙인다 (SQL 주입 여지 없음). 빈 목록이 되지 않게 어떤 차선도 아닌 값을 하나 둔다
    let list = busy
        .iter()
        .chain(std::iter::once(&i64::MIN))
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT j.job_id, j.asset_id, j.folder_id, j.priority, j.attempts,
                COALESCE(f.volume_id, -COALESCE(j.folder_id, 0)) AS lane
         FROM jobs j LEFT JOIN managed_folders f ON f.folder_id = j.folder_id
         WHERE j.kind = 'scan' AND j.status = 'pending' AND j.not_before <= ?1 AND j.priority <= ?2
           AND lane NOT IN ({list})
         ORDER BY j.priority, j.job_id LIMIT 1"
    );
    tx.query_row(&sql, params![now, max_priority], |r| {
        Ok((
            Job {
                job_id: r.get(0)?,
                kind: JobKind::Scan,
                asset_id: r.get(1)?,
                folder_id: r.get(2)?,
                priority: r.get(3)?,
                attempts: r.get(4)?,
            },
            r.get(5)?,
        ))
    })
    .optional()
}
