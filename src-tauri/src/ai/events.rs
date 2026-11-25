//! AI 이벤트 — `models://progress`(models.rs)와 `ai://analysis`(자산별 분석 상태 변화, 스로틀).
//!
//! `ai://analysis` 는 UI 폴링의 대체 경로다. 실행기를 감싼 [`NotifyingExecutor`] 가 작업 시작/종료 때 자산을
//! 표시하고, 250ms 마다 [`AnalysisNotifier::flush`] 가 **바뀐** 상태만 최대 100자산씩 보낸다.
//! 많은 자산을 한꺼번에 등록하는 일괄 분석은 자산별 이벤트를 만들지 않는다 — UI 가 폴링/재조회로 따라간다.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use mam_core::ai_jobs::{AiExecutor, StepStatus, list_status};
use mam_core::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome};
use mam_db::DbPool;
use serde::Serialize;

pub const MODELS_PROGRESS_EVENT: &str = "models://progress";
pub const ANALYSIS_EVENT: &str = "ai://analysis";
/// 한 번에 보내는 자산 수 상한
pub const FLUSH_MAX: usize = 100;
/// 쌓아 두는 변경 자산 수 상한 (넘으면 버린다 — 폴링이 보완)
const DIRTY_MAX: usize = 10_000;

pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// Tauri `AppHandle` 은 setup 이후에야 생기므로 나중에 연결한다. 연결 전 이벤트는 버린다.
#[derive(Default)]
pub struct LateSink(OnceLock<Arc<dyn EventSink>>);

impl LateSink {
    pub fn set(&self, sink: Arc<dyn EventSink>) {
        let _ = self.0.set(sink);
    }
}

impl EventSink for LateSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        if let Some(s) = self.0.get() {
            s.emit(event, payload);
        }
    }
}

pub struct TauriSink(pub tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        if let Err(e) = self.0.emit(event, payload) {
            tracing::debug!(%e, event, "cannot emit event");
        }
    }
}

/// `ai://analysis` 항목.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisChange {
    pub asset_id: i64,
    pub status: Vec<StepStatus>,
}

/// `ai://analysis` 페이로드.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalysisEvent {
    pub changes: Vec<AnalysisChange>,
}

#[derive(Default)]
struct Inner {
    dirty: BTreeSet<i64>,
    active: HashSet<i64>,
    last: HashMap<i64, Vec<StepStatus>>,
}

#[derive(Default)]
pub struct AnalysisNotifier {
    inner: Mutex<Inner>,
}

impl AnalysisNotifier {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn mark(&self, asset_id: i64) {
        let mut g = self.lock();
        if g.dirty.len() < DIRTY_MAX {
            g.dirty.insert(asset_id);
        }
    }

    fn start(&self, asset_id: i64) {
        let mut g = self.lock();
        g.active.insert(asset_id);
        if g.dirty.len() < DIRTY_MAX {
            g.dirty.insert(asset_id);
        }
    }

    fn finish(&self, asset_id: i64) {
        let mut g = self.lock();
        g.active.remove(&asset_id);
        if g.dirty.len() < DIRTY_MAX {
            g.dirty.insert(asset_id);
        }
    }

    /// 보낼 변경을 모아 발행한다. 보낸 자산 수를 돌려준다.
    pub fn flush(&self, pool: &DbPool, sink: &dyn EventSink) -> usize {
        let ids: Vec<i64> = {
            let g = self.lock();
            g.dirty
                .iter()
                .chain(g.active.iter())
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .take(FLUSH_MAX)
                .collect()
        };
        if ids.is_empty() {
            return 0;
        }
        let statuses = pool.read(|c| {
            Ok(ids
                .iter()
                .filter_map(|id| list_status(c, *id).ok().map(|s| (*id, s)))
                .collect::<Vec<_>>())
        });
        let Ok(statuses) = statuses else { return 0 };
        let mut changes = Vec::new();
        {
            let mut g = self.lock();
            for (id, status) in statuses {
                g.dirty.remove(&id);
                if g.last.get(&id) != Some(&status) {
                    g.last.insert(id, status.clone());
                    changes.push(AnalysisChange {
                        asset_id: id,
                        status,
                    });
                }
                if !g.active.contains(&id) {
                    // 끝난 자산의 마지막 상태는 한 번만 보내면 된다 — 기억을 비워 메모리가 늘지 않게 한다
                    g.last.remove(&id);
                }
            }
        }
        let n = changes.len();
        if n > 0
            && let Ok(v) = serde_json::to_value(AnalysisEvent { changes })
        {
            sink.emit(ANALYSIS_EVENT, v);
        }
        n
    }
}

/// `AiExecutor` 를 감싸 작업 시작/종료를 알린다.
pub struct NotifyingExecutor {
    inner: AiExecutor,
    notifier: Arc<AnalysisNotifier>,
}

impl NotifyingExecutor {
    pub fn new(inner: AiExecutor, notifier: Arc<AnalysisNotifier>) -> Self {
        Self { inner, notifier }
    }
}

impl JobExecutor for NotifyingExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let Some(id) = job.asset_id else {
            return self.inner.execute(job, ctx);
        };
        self.notifier.start(id);
        let out = self.inner.execute(job, ctx);
        self.notifier.finish(id);
        out
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
