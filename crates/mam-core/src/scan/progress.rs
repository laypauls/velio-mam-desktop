//! 스캔 진행 상태 — 진행률 이벤트와 멈춤 감시(3분 넘게 같은 경로를 붙들고 있으면 `stalled`)가 읽는다.
//!
//! 스캔은 볼륨별 차선으로 동시에 여러 개 돌 수 있어(결정 E) 폴더별로 한 칸씩 둔다. 시각은 주입 시계(`now`)로만 받는다.

use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

/// 같은 경로를 이만큼(ms) 이상 붙들고 있으면 멈춘 것으로 본다 (watch_folder `scanWatchdog` 와 같은 3분, 경계 포함)
pub const STALL_AFTER_MS: i64 = 180_000;

/// 스캔 진행 상태 (진행률 이벤트용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanProgress {
    pub folder_id: i64,
    pub folder_path: String,
    pub processed: u64,
    /// 지금 읽고 있는(붙들고 있는) 경로
    pub held_path: Option<String>,
    /// `held_path` 를 붙들기 시작한(또는 마지막으로 진행이 있던) 시각 (UTC ms)
    pub held_since: i64,
}

/// 폴더 상태 `scanStall` (프론트 계약 `ScanStall`) — 스캔이 한 곳에서 오래 멈춰 있다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStall {
    pub at_path: String,
    pub since: i64,
}

impl ScanProgress {
    /// `now` 에 멈춘 상태인가 (붙든 지 정확히 3분이면 멈춤, 2분 59초면 아직).
    pub fn stall_at(&self, now: i64) -> Option<ScanStall> {
        let path = self.held_path.as_ref()?;
        (now.saturating_sub(self.held_since) >= STALL_AFTER_MS).then(|| ScanStall {
            at_path: path.clone(),
            since: self.held_since,
        })
    }
}

#[derive(Default)]
pub struct ScanProgressCell(Mutex<Vec<ScanProgress>>);

impl ScanProgressCell {
    fn guard(&self) -> MutexGuard<'_, Vec<ScanProgress>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn start(&self, folder_id: i64, folder_path: String, processed: u64, now: i64) {
        let mut g = self.guard();
        g.retain(|p| p.folder_id != folder_id);
        g.push(ScanProgress {
            held_path: Some(folder_path.clone()),
            folder_id,
            folder_path,
            processed,
            held_since: now,
        });
    }

    /// 처리한 건수를 더한다. 진행이 있었으므로 멈춤 시계도 다시 시작한다.
    pub fn add(&self, folder_id: i64, n: u64, now: i64) {
        if let Some(p) = self.guard().iter_mut().find(|p| p.folder_id == folder_id) {
            p.processed += n;
            if n > 0 {
                p.held_since = now;
            }
        }
    }

    /// 지금 붙들고 있는 경로를 기록한다. 경로가 바뀌면 시작 시각을 `now` 로 옮기고, 같으면 그대로 둔다.
    pub fn hold(&self, folder_id: i64, path: &str, now: i64) {
        if let Some(p) = self.guard().iter_mut().find(|p| p.folder_id == folder_id)
            && p.held_path.as_deref() != Some(path)
        {
            p.held_path = Some(path.to_owned());
            p.held_since = now;
        }
    }

    /// 붙들고 있는 파일에서 바이트가 읽혔다 — 큰 파일 하나를 해시하는 동안에도 경로는 그대로지만 멈춘 것이 아니다.
    /// 멈춤 시계만 다시 시작한다 (처리 건수는 건드리지 않는다).
    pub fn bytes_read(&self, folder_id: i64, now: i64) {
        if let Some(p) = self.guard().iter_mut().find(|p| p.folder_id == folder_id) {
            p.held_since = now;
        }
    }

    pub fn clear(&self, folder_id: i64) {
        self.guard().retain(|p| p.folder_id != folder_id);
    }

    /// 진행 중인 스캔 하나 (먼저 시작한 것) — `job_summary.scan` 은 하나만 싣는다
    pub fn get(&self) -> Option<ScanProgress> {
        self.guard().first().cloned()
    }

    pub fn all(&self) -> Vec<ScanProgress> {
        self.guard().clone()
    }
}
