//! 스캔 멈춤 감시 — 스캔이 3분 넘게 같은 경로를 붙들고 있으면 폴더 상태를 `stalled { at_path, since }` 로 표시한다.
//!
//! 판정은 스캔 진행 상태(`ScanProgress::held_path/held_since`)와 주입 시계로만 한다 — 상태는 저장하지 않고 읽을 때 계산하므로
//! 점검 주기(30초)를 기다리지 않고 `list_folders` 도 같은 값을 본다. 로그는 점검(`supervise_once`)이 남긴다:
//! 멈춘 동안 10분마다 1회, 풀리면(경로가 바뀌거나 스캔이 끝나면) 1회.

use std::collections::HashMap;

use super::{Ingest, lock};
use crate::scan::ScanStall;

/// 멈춘 동안 같은 폴더를 로그로 다시 알리는 간격
pub const STALL_LOG_EVERY_MS: i64 = 600_000;

/// 점검 1회가 낸 멈춤 관련 알림 (시험·로그용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StallEvent {
    /// 멈춤 시작 또는 계속 중 (10분마다)
    Stalled { folder_id: i64, stall: ScanStall },
    /// 풀렸다 — 한 번만
    Recovered { folder_id: i64 },
}

/// 폴더별 마지막 로그 시각 (멈춘 폴더만 들어 있다)
#[derive(Default)]
pub(super) struct StallLog(std::sync::Mutex<HashMap<i64, i64>>);

impl Ingest {
    /// 지금 멈춰 있는 스캔 (폴더 id -> 멈춘 곳)
    pub fn scan_stalls(&self) -> HashMap<i64, ScanStall> {
        let now = self.env.clock.now_ms();
        self.env
            .progress
            .all()
            .into_iter()
            .filter_map(|p| p.stall_at(now).map(|s| (p.folder_id, s)))
            .collect()
    }

    /// 멈춤 상태를 로그에 반영한다 (점검이 부른다). 새로 알린 것만 돌려준다.
    pub fn check_stalls(&self) -> Vec<StallEvent> {
        let now = self.env.clock.now_ms();
        let current = self.scan_stalls();
        let mut logged = lock(&self.stall_log.0);
        let mut events = Vec::new();
        for (folder_id, stall) in &current {
            let due = logged
                .get(folder_id)
                .is_none_or(|at| now.saturating_sub(*at) >= STALL_LOG_EVERY_MS);
            if due {
                logged.insert(*folder_id, now);
                tracing::warn!(
                    folder_id,
                    stalled_secs = now.saturating_sub(stall.since) / 1000,
                    "scan is stuck on one path; the volume may not be responding"
                );
                events.push(StallEvent::Stalled {
                    folder_id: *folder_id,
                    stall: stall.clone(),
                });
            }
        }
        let recovered: Vec<i64> = logged
            .keys()
            .copied()
            .filter(|id| !current.contains_key(id))
            .collect();
        for folder_id in recovered {
            logged.remove(&folder_id);
            tracing::info!(folder_id, "scan is moving again");
            events.push(StallEvent::Recovered { folder_id });
        }
        events
    }
}
