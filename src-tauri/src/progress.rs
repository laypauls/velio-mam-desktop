//! `jobs://progress` 이벤트 발행 (P2-2, D8) — 최대 4Hz.
//!
//! 250ms 마다 한 번만 상태를 보고, 직전에 보낸 값과 같으면 보내지 않는다. 한가할 때는 상태 변화 번호(generation)
//! 비교만 하므로 DB 를 읽지 않는다. 작업이 진행 중일 때는 진행 건수가 바뀌므로 매 틱 요약을 만든다.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mam_core::ingest::{Ingest, JobSummary};
use mam_core::scan::IngestError;
use tauri::{AppHandle, Emitter};

/// 계약 `JOBS_PROGRESS_EVENT`
pub const JOBS_PROGRESS_EVENT: &str = "jobs://progress";
/// 발행 주기 = 4Hz 상한
pub const TICK: Duration = Duration::from_millis(250);

pub trait SummarySource {
    fn generation(&self) -> u64;
    fn summary(&self) -> Result<JobSummary, IngestError>;
}

impl SummarySource for Ingest {
    fn generation(&self) -> u64 {
        Ingest::generation(self)
    }
    fn summary(&self) -> Result<JobSummary, IngestError> {
        Ingest::summary(self)
    }
}

#[derive(Default)]
pub struct ProgressGate {
    last_generation: Option<u64>,
    last: Option<JobSummary>,
}

impl ProgressGate {
    /// 틱 1회. 보낼 요약이 있으면 돌려준다.
    pub fn poll(&mut self, source: &dyn SummarySource) -> Option<JobSummary> {
        let generation = source.generation();
        let busy = self
            .last
            .as_ref()
            .is_some_and(|s| s.scan.is_some() || s.counts.values().any(|c| c.running > 0));
        if self.last_generation == Some(generation) && !busy {
            return None;
        }
        self.last_generation = Some(generation);
        let now = source.summary().ok()?;
        if self.last.as_ref() == Some(&now) {
            return None;
        }
        self.last = Some(now.clone());
        Some(now)
    }
}

/// 발행 스레드를 시작한다. `stop` 이 켜지면 끝난다.
pub fn spawn(app: AppHandle, ingest: Arc<Ingest>, stop: Arc<AtomicBool>) {
    let spawned = std::thread::Builder::new()
        .name("mam-progress".into())
        .spawn(move || {
            let mut gate = ProgressGate::default();
            while !stop.load(Ordering::Relaxed) {
                if let Some(summary) = gate.poll(&*ingest)
                    && let Err(e) = app.emit(JOBS_PROGRESS_EVENT, summary)
                {
                    tracing::warn!(%e, "cannot emit progress event");
                }
                std::thread::sleep(TICK);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(%e, "cannot start progress emitter");
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy (변화 시 1회) / Boundary (한가할 때 DB 미접근) / Business rule (진행 중 갱신)
    use std::cell::Cell;
    use std::collections::BTreeMap;

    use mam_core::jobs::{JobCounts, JobKind};

    use super::*;

    struct Fake {
        generation: Cell<u64>,
        processed: Cell<Option<u64>>,
        reads: Cell<u32>,
    }

    impl SummarySource for Fake {
        fn generation(&self) -> u64 {
            self.generation.get()
        }
        fn summary(&self) -> Result<JobSummary, IngestError> {
            self.reads.set(self.reads.get() + 1);
            let mut counts: BTreeMap<JobKind, JobCounts> = JobKind::ALL
                .into_iter()
                .map(|k| (k, JobCounts::default()))
                .collect();
            let scan = self.processed.get().map(|p| {
                if let Some(c) = counts.get_mut(&JobKind::Scan) {
                    c.running = 1;
                }
                mam_core::ingest::ScanProgressView {
                    folder_id: 1,
                    folder_path: r"D:\media".into(),
                    processed: p,
                    total: None,
                }
            });
            Ok(JobSummary {
                paused: false,
                paused_kinds: vec![],
                counts,
                scan,
                total_assets: 0,
            })
        }
    }

    #[test]
    fn t_ipc_12_progress_is_sent_only_on_change_and_idle_ticks_do_not_read_state() {
        // Arrange
        let src = Fake {
            generation: Cell::new(1),
            processed: Cell::new(None),
            reads: Cell::new(0),
        };
        let mut gate = ProgressGate::default();

        // Act / Assert: 첫 틱은 현재 상태를 한 번 보낸다
        assert!(gate.poll(&src).is_some());
        // 한가하고 변화 없음 -> 보내지 않고 상태도 읽지 않는다
        let reads = src.reads.get();
        for _ in 0..5 {
            assert!(gate.poll(&src).is_none());
        }
        assert_eq!(src.reads.get(), reads);
        // 상태 변화 번호가 오르면 다시 읽고, 내용이 같으면 보내지 않는다
        src.generation.set(2);
        assert!(gate.poll(&src).is_none());
        assert_eq!(src.reads.get(), reads + 1);
    }

    #[test]
    fn t_ipc_12b_running_scan_reports_each_progress_change_once_per_tick() {
        let src = Fake {
            generation: Cell::new(1),
            processed: Cell::new(Some(0)),
            reads: Cell::new(0),
        };
        let mut gate = ProgressGate::default();
        assert!(gate.poll(&src).is_some());

        // 진행 중에는 generation 이 그대로여도 건수가 바뀌면 보낸다 (틱당 최대 1회 = 4Hz)
        src.processed.set(Some(120));
        let sent = gate.poll(&src).unwrap();
        assert_eq!(sent.scan.unwrap().processed, 120);
        assert!(gate.poll(&src).is_none(), "unchanged -> nothing to send");

        // 스캔이 끝나면(None) 마지막 상태가 한 번 나가고 이후 조용해진다
        src.processed.set(None);
        assert!(gate.poll(&src).unwrap().scan.is_none());
        assert!(gate.poll(&src).is_none());
        assert!(TICK >= Duration::from_millis(250), "4Hz upper bound");
    }

    #[test]
    fn t_ipc_13_event_name_matches_the_frontend_contract() {
        assert_eq!(JOBS_PROGRESS_EVENT, "jobs://progress");
    }
}
