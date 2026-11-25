//! 변경 대기열 — 감시(와 스캔)가 보고한 "확인할 경로"를 SQLite(`watch_pending`)에 **먼저 기록하고** 처리한다.
//!
//! ```text
//! ChangeSink ─► mpsc ─► 기록기(writer.rs): 0.25초마다 묶어 1 트랜잭션 UPSERT (경로당 1행, Priority::Low)
//!                          ▼
//!             처리기(processor.rs): due_at <= now 인 행을 기한 순 최대 N개
//!               ├ 사라짐 ─────────────► apply_path (missing 표시) -> 행 삭제
//!               ├ 표본이 바뀜 ─────────► 표본 갱신, due_at = now + 간격   (settle.rs: SettleGate)
//!               ├ 표본 N회 연속 같음 ──► apply_path -> 행 삭제  (배타적 열기 옵션이 켜져 있으면 그것도 통과해야)
//!               └ 실패 ────────────────► attempts+1, 백오프 -> 5회에 scan_errors 기록 후 삭제 (재스캔이 맞춘다)
//! 시작: 남은 행은 표본을 버리고 바로 다시 확인 / 종료: mpsc 에 남은 것을 최대 2초 마저 기록 / 정상 종료 표식(marker.rs)
//! ```
//!
//! `apply_path` 가 이벤트 종류를 믿지 않고 **실제 파일 상태**로 판단(멱등)하므로 순서·이력은 필요 없고 경로당 1행이면 된다.
//! 처리기의 `apply_path` 와 행 삭제는 한 트랜잭션이 아니다 — 중간에 죽으면 다시 반영하지만 멱등이라 무해하다.

mod marker;
mod remote;
mod processor;
mod settle;
pub(crate) mod store;
mod writer;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mam_db::Priority;

use super::Ingest;
use crate::scan::{IngestError, load_all};
use crate::watch::{ChangeDetector, OsEvents, WatchTarget};

pub use processor::PendingPassReport;
pub use remote::DbCommitter;
pub use settle::{ExclusiveOutcome, SettleGate};
pub use store::{PendingKind, PendingRow};

/// 전체 스캔이 "지금 막 바뀐 파일"로 보고 안정화 게이트에 넘기는 수정 시각 창 (결정 I, PC 시계 기준)
pub const SCAN_RECENT_GUARD_MS: i64 = 60_000;
/// `apply_path` 가 이만큼 실패하면 `scan_errors` 에 남기고 행을 지운다 (재스캔이 맞춘다)
pub const MAX_ATTEMPTS: i64 = 5;

#[derive(Debug, Clone)]
pub struct PendingConfig {
    pub gate: SettleGate,
    /// 기록기가 보고를 묶어 쓰는 간격 (실시간)
    pub write_every: Duration,
    /// 처리기 점검 간격 (실시간)
    pub process_every: Duration,
    /// 처리기가 한 번에 꺼내는 최대 행 수
    pub batch: usize,
    /// 폴더 하나의 최대 대기 행 수 — 닿으면 새 경로를 받지 않고 재스캔 1회로 바꾼다
    pub max_rows_per_folder: usize,
    /// 종료 시 mpsc 에 남은 보고를 마저 기록하는 최대 시간 (취소된 토큰과 별개의 기한)
    pub drain: Duration,
    /// 스캔이 이 창(ms) 안에 수정된(또는 미래 시각인) 파일을 반영하지 않고 게이트에 넘긴다. 0 = 끔.
    /// 기본값은 0 — 제품 배선(`src-tauri`)만 [`SCAN_RECENT_GUARD_MS`] 로 켠다 (시험은 방금 만든 파일을 바로 스캔한다)
    pub recent_guard_ms: i64,
}

impl Default for PendingConfig {
    fn default() -> Self {
        Self {
            gate: SettleGate::default(),
            write_every: Duration::from_millis(250),
            process_every: Duration::from_millis(250),
            batch: 200,
            max_rows_per_folder: 50_000,
            drain: Duration::from_secs(2),
            recent_guard_ms: 0,
        }
    }
}

/// `Ingest` 가 쥐는 대기열 런타임 상태.
#[derive(Default)]
pub(super) struct PendingState {
    /// 상한에 막혀 재스캔으로 바꾼 폴더 — 행이 다시 상한 아래로 내려가면 풀린다 (재스캔 반복 예약 방지)
    overflowed: Mutex<HashSet<i64>>,
    /// "복사가 끝나지 않음" 경고를 이미 낸 행 (프로세스당 1회)
    warned_overdue: Mutex<HashSet<(i64, String)>>,
    /// 이 실행이 비정상 종료 표식을 남겼다 (정상 종료에서 지운다)
    run_marked: AtomicBool,
}

impl Ingest {
    /// 시작 복구: 남은 행의 표본을 버리고 바로 다시 확인하게 한다. 되돌린 행 수.
    pub fn recover_pending(&self) -> Result<usize, IngestError> {
        let now = self.env.clock.now_ms();
        let n = self
            .env
            .write(Priority::Low, move |c| store::recover_all(c, now))?;
        if n > 0 {
            tracing::info!(rows = n, "recovered pending watch changes");
        }
        Ok(n)
    }

    /// 변경 1건을 mpsc 를 거치지 않고 지금 기록한다 (시험·진단용, 그리고 `Subtree` 를 보고하는 감지기용).
    /// 쓰기와 같은 경로(경로당 1행 UPSERT·폴더 상한)를 쓴다.
    pub fn queue_watch_change(
        &self,
        folder_id: i64,
        path: &Path,
        kind: PendingKind,
    ) -> Result<(), IngestError> {
        let source = self.source_of(folder_id);
        let Some(item) = store::Incoming::new(folder_id, path, kind, source) else {
            return Err(IngestError::InvalidInput("path"));
        };
        let report = self.enqueue_incoming(vec![item])?;
        self.on_overflow(&report.overflowed);
        Ok(())
    }

    /// 폴더의 반영 대기 행 수.
    pub fn pending_count(&self, folder_id: i64) -> Result<i64, IngestError> {
        self.env
            .pool
            .read(|c| Ok(store::count_for_folder(c, folder_id)))?
    }

    /// 실행 시작 표식 (결정 Q). 직전 실행이 정상 종료하지 못했으면(표식이 남아 있으면) **실시간 감시(`os_events`)로 동작할 폴더**에
    /// 재스캔 1회를 예약하고 예약한 폴더를 돌려준다. 실시간 감시는 앱이 꺼져 있는 동안의 변경을 볼 수 없다.
    pub fn begin_run(&self) -> Result<Vec<i64>, IngestError> {
        let now = self.env.clock.now_ms();
        let abnormal = self
            .env
            .write(Priority::High, move |c| marker::begin(c, now))?;
        self.pending.run_marked.store(true, Ordering::Relaxed);
        if !abnormal {
            return Ok(Vec::new());
        }
        tracing::warn!(
            "previous run did not shut down cleanly; rescanning real-time watched folders"
        );
        let mut scheduled = Vec::new();
        for f in self
            .env
            .pool
            .read(|c| Ok(load_all(c)))??
            .iter()
            .filter(|f| f.enabled && f.online)
        {
            let plan = self.watchers.registry().plan(
                &WatchTarget::of(f),
                f.watch_mode.as_deref(),
                f.watch_options.as_deref(),
            );
            let realtime = plan
                .candidates
                .first()
                .is_some_and(|c| c.detector.id() == OsEvents.id());
            if !realtime {
                continue;
            }
            match self.enqueue_scan(f.folder_id, false) {
                Ok(true) => scheduled.push(f.folder_id),
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(%e, folder_id = f.folder_id, "cannot schedule rescan after unclean shutdown")
                }
            }
        }
        Ok(scheduled)
    }

    /// 정상 종료 — 시작 표식을 지운다. 이 실행이 표식을 남기지 않았으면 아무것도 하지 않는다.
    pub fn end_run(&self) {
        if self.pending.run_marked.swap(false, Ordering::Relaxed)
            && let Err(e) = self.env.write(Priority::High, |c| marker::clear(c))
        {
            tracing::warn!(%e, "cannot clear the run marker");
        }
    }

    /// 보고를 한 트랜잭션에 기록한다 (디바운스 끝 = 지금 + `debounce_ms`).
    pub(super) fn enqueue_incoming(
        &self,
        items: Vec<store::Incoming>,
    ) -> Result<store::UpsertReport, IngestError> {
        let now = self.env.clock.now_ms();
        let due = now + self.config.debounce_ms;
        let cap = self.config.pending.max_rows_per_folder;
        // 청크마다 별도 쓰기 작업 — 사이에 사용자 편집(High)이 끼어들 수 있다 (store::MAX_ROWS_PER_TX)
        store::upsert_chunked(&items, store::MAX_ROWS_PER_TX, |part, _last| {
            let part = part.to_vec();
            self.env.write(Priority::Low, move |c| store::upsert_batch(c, &part, now, due, cap))
        })
    }

    /// 보고한 감지기 ID (진단용 `source`). 감시 중이 아니면 `watch`.
    pub(super) fn source_of(&self, folder_id: i64) -> &'static str {
        self.watchers
            .status(folder_id)
            .map_or("watch", |s| s.active)
    }

    /// 상한에 막힌 폴더를 재스캔 1회로 바꾼다 (폴더당 상한 아래로 내려갈 때까지 한 번만).
    pub(super) fn on_overflow(&self, folders: &[i64]) {
        for &folder_id in folders {
            if super::lock(&self.pending.overflowed).insert(folder_id) {
                tracing::warn!(
                    folder_id,
                    "too many pending watch changes; falling back to a rescan"
                );
                self.schedule_rescan_quiet(folder_id);
            }
        }
    }
}
