//! 폴더별 감시 관리 — 감지기 선택·폴백·시작·중지·해제 대기. (옛 `Watchers` 대체)

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::SharedClock;

use super::detector::{DetectError, WatchHandle, WatchTarget};
use super::health::{HealthBoard, SourceHealth};
use super::registry::{DetectorRegistry, FallbackReason};
use super::release::{ReleaseTracker, SinkId};
use super::sink::{ChangeSink, WatchMsg};

/// 지금 실제로 동작 중인 감지기와 (지정한 방식이 아닐 때) 폴백 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchStatus {
    pub active: &'static str,
    pub fallback: Option<FallbackReason>,
}

pub struct StartOutcome {
    pub status: WatchStatus,
    pub started_now: bool,
    /// 시작에 실패한 마지막 후보의 오류 (폴백했거나 모두 실패했을 때)
    pub last_error: Option<DetectError>,
}

struct Entry {
    handle: Box<dyn WatchHandle>,
    sink: SinkId,
    status: WatchStatus,
    /// 어떤 후보의 `start` 가 실패했다 — 표시 사유와 별개로 supervisor 가 다시 시도한다
    start_failed: bool,
}

pub struct WatchManager {
    tx: Sender<WatchMsg>,
    registry: Arc<DetectorRegistry>,
    active: Mutex<HashMap<i64, Entry>>,
    tracker: Arc<ReleaseTracker>,
    health: Arc<HealthBoard>,
}

impl WatchManager {
    /// 시스템 시계를 쓴다 — 신호원 상태의 시각을 주입하려면 [`Self::with_clock`].
    pub fn new(tx: Sender<WatchMsg>, registry: Arc<DetectorRegistry>) -> Self {
        Self::with_clock(tx, registry, Arc::new(mam_db::SystemClock))
    }

    pub fn with_clock(
        tx: Sender<WatchMsg>,
        registry: Arc<DetectorRegistry>,
        clock: SharedClock,
    ) -> Self {
        Self {
            tx,
            registry,
            active: Mutex::new(HashMap::new()),
            tracker: Arc::default(),
            health: Arc::new(HealthBoard::new(clock)),
        }
    }

    fn map(&self) -> MutexGuard<'_, HashMap<i64, Entry>> {
        self.active.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 맵 잠금을 쥔 채로 부른다: sink 를 닫아 `wait_released` 가 이 핸들의 토큰을 기다리게 한 뒤 핸들을 돌려준다.
    /// 호출자가 잠금을 놓은 뒤 핸들을 버린다 (엔트리가 사라진 순간과 sink 가 닫히는 순간 사이에 틈이 없다).
    fn detach(&self, e: Entry) -> Box<dyn WatchHandle> {
        self.tracker.close(e.sink);
        e.handle
    }

    /// 감시를 시작한다. 정착한 엔트리가 있으면 아무것도 하지 않는다(`started_now = false`).
    /// 아니면 옛 엔트리를 치우고 후보를 순서대로 시도한다 — 시작 실패는 다음 후보로 넘어가고 `scan_only` 는 항상 성공한다.
    pub fn start(
        &self,
        target: &WatchTarget,
        mode: Option<&str>,
        raw_options: Option<&str>,
    ) -> StartOutcome {
        let old = {
            let mut map = self.map();
            if let Some(e) = map.get(&target.folder_id)
                && !e.start_failed
            {
                return StartOutcome {
                    status: e.status.clone(),
                    started_now: false,
                    last_error: None,
                };
            }
            map.remove(&target.folder_id).map(|e| self.detach(e))
        };
        drop(old);
        let plan = self.registry.plan(target, mode, raw_options);
        let mut reason = plan.reason;
        let mut last_error = None;
        for c in plan.candidates {
            let sink_id = self.tracker.open();
            let sink = ChangeSink::new(
                target.folder_id,
                self.tx.clone(),
                sink_id,
                Arc::clone(&self.tracker),
                Arc::clone(&self.health),
            );
            self.health.begin(target.folder_id, sink_id);
            match c.detector.start(target, &c.options, sink) {
                Ok(handle) => {
                    let status = WatchStatus {
                        active: c.detector.id(),
                        fallback: reason,
                    };
                    let displaced = {
                        let mut map = self.map();
                        map.insert(
                            target.folder_id,
                            Entry {
                                handle,
                                sink: sink_id,
                                status: status.clone(),
                                start_failed: last_error.is_some(),
                            },
                        )
                        .map(|e| self.detach(e))
                    };
                    drop(displaced);
                    return StartOutcome {
                        status,
                        started_now: true,
                        last_error,
                    };
                }
                Err(e) => {
                    // 실패한 시작이 남긴 토큰(이미 만든 notify 클로저 등)도 기다릴 수 있게 sink 는 닫기만 한다
                    self.tracker.close(sink_id);
                    self.health.end_if(target.folder_id, sink_id);
                    reason = reason.or(Some(FallbackReason::StartFailed));
                    last_error = Some(e);
                }
            }
        }
        // scan_only 가 항상 성공하므로 도달하지 않는다 — 엔트리를 만들지 않아 supervisor 가 다시 시도한다
        tracing::error!(
            folder_id = target.folder_id,
            "no change detector could start"
        );
        StartOutcome {
            status: WatchStatus {
                active: "scan_only",
                fallback: reason,
            },
            started_now: false,
            last_error,
        }
    }

    /// 엔트리를 치우며 같은 잠금 안에서 sink 를 닫고, 잠금을 놓은 뒤 핸들을 버린다 (동기).
    pub fn stop(&self, folder_id: i64) {
        self.health.end(folder_id);
        let old = {
            let mut map = self.map();
            map.remove(&folder_id).map(|e| self.detach(e))
        };
        drop(old);
    }

    pub fn stop_all(&self) {
        self.health.clear();
        let all: Vec<_> = {
            let mut map = self.map();
            map.drain().map(|(_, e)| self.detach(e)).collect()
        };
        drop(all);
    }

    /// 어떤 감지기든(`scan_only` 포함) 엔트리가 있는가.
    pub fn is_watching(&self, folder_id: i64) -> bool {
        self.map().contains_key(&folder_id)
    }

    /// 엔트리가 있고 시작 실패 폴백이 아니다 — false 면 supervisor 가 다시 시도한다.
    pub fn is_settled(&self, folder_id: i64) -> bool {
        self.map().get(&folder_id).is_some_and(|e| !e.start_failed)
    }

    pub fn status(&self, folder_id: i64) -> Option<WatchStatus> {
        self.map().get(&folder_id).map(|e| e.status.clone())
    }

    /// 신호원 상태 — 감시 중이 아니면 None.
    pub fn health(&self, folder_id: i64) -> Option<SourceHealth> {
        self.health.get(folder_id)
    }

    /// 멈춘 감시들의 디렉터리 핸들이 실제로 닫힐 때까지 기다린다 (감시 중인 것은 기다리지 않는다).
    pub fn wait_released(&self, timeout: Duration) -> bool {
        self.tracker.wait_released(timeout)
    }

    pub fn registry(&self) -> &DetectorRegistry {
        &self.registry
    }

    #[cfg(test)]
    pub(crate) fn live_tokens(&self) -> usize {
        self.tracker.live_tokens()
    }
}
