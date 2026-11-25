//! 신호원 상태 — 감지기가 sink 로 "신호를 받았다/문제가 있다" 를 알리면 폴더별로 쥐고 `list_folders` 가 읽는다.
//!
//! 상태는 저장하지 않는다(재시작하면 새로 시작). 시각은 주입 시계로만 읽고, 기간이 정해진 `Degraded` 는 읽을 때 만료를 판정한다
//! (점검을 기다리지 않는다). 낡은 sink(교체·중지된 감시)의 보고는 `SinkId` 가 달라 무시한다.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use super::release::SinkId;
use crate::SharedClock;

/// 폴더 변경 신호원의 상태 (프론트 계약 `SourceHealth`). 실시간이 아닌 `scan_only` 는 아무것도 보고하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum SourceHealth {
    /// 정상 — `last_signal_at` 은 마지막으로 변경 신호를 받은 시각 (UTC ms, 아직 없으면 None)
    #[serde(rename_all = "camelCase")]
    Ok { last_signal_at: Option<i64> },
    /// 재연결 중·응답 느림·이벤트 넘침 — 놓친 변경은 재스캔이 보완한다. 사유는 번역 키 `watch.health.*`
    #[serde(rename_all = "camelCase")]
    Degraded {
        since: i64,
        reason_key: &'static str,
    },
}

struct Degraded {
    since: i64,
    reason_key: &'static str,
    /// 이 시각(ms) 이후에는 정상으로 돌아온다 (None = `recovered` 가 불릴 때까지)
    until: Option<i64>,
}

struct Slot {
    sink: SinkId,
    last_signal_at: Option<i64>,
    degraded: Option<Degraded>,
}

pub(crate) struct HealthBoard {
    clock: SharedClock,
    slots: Mutex<HashMap<i64, Slot>>,
}

impl HealthBoard {
    pub(crate) fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<i64, Slot>> {
        self.slots.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 이 sink 가 폴더의 현재 신호원이 된다 (상태는 새로 시작).
    pub(crate) fn begin(&self, folder_id: i64, sink: SinkId) {
        self.slots().insert(
            folder_id,
            Slot {
                sink,
                last_signal_at: None,
                degraded: None,
            },
        );
    }

    pub(crate) fn end(&self, folder_id: i64) {
        self.slots().remove(&folder_id);
    }

    /// 시작에 실패한 sink 의 자리만 치운다 (이미 다른 sink 가 이어받았으면 건드리지 않는다).
    pub(crate) fn end_if(&self, folder_id: i64, sink: SinkId) {
        let mut g = self.slots();
        if g.get(&folder_id).is_some_and(|s| s.sink == sink) {
            g.remove(&folder_id);
        }
    }

    pub(crate) fn clear(&self) {
        self.slots().clear();
    }

    fn with(&self, folder_id: i64, sink: SinkId, f: impl FnOnce(&mut Slot, i64)) {
        let now = self.clock.now_ms();
        if let Some(s) = self.slots().get_mut(&folder_id).filter(|s| s.sink == sink) {
            f(s, now);
        }
    }

    pub(crate) fn signal(&self, folder_id: i64, sink: SinkId) {
        self.with(folder_id, sink, |s, now| s.last_signal_at = Some(now));
    }

    /// `hold_ms` 가 있으면 그 시간 뒤 저절로 정상으로 돌아온다. 이미 저하 중이면 시작 시각을 유지하고 사유·기한만 갱신한다.
    pub(crate) fn degrade(
        &self,
        folder_id: i64,
        sink: SinkId,
        reason_key: &'static str,
        hold_ms: Option<i64>,
    ) {
        self.with(folder_id, sink, |s, now| {
            let still = s
                .degraded
                .as_ref()
                .filter(|d| d.until.is_none_or(|u| now < u))
                .map(|d| d.since);
            s.degraded = Some(Degraded {
                since: still.unwrap_or(now),
                reason_key,
                until: hold_ms.map(|h| now.saturating_add(h)),
            });
        });
    }

    pub(crate) fn recover(&self, folder_id: i64, sink: SinkId) {
        self.with(folder_id, sink, |s, _| s.degraded = None);
    }

    /// 지금 상태. 신호원이 없으면(감시 중 아님) None.
    pub(crate) fn get(&self, folder_id: i64) -> Option<SourceHealth> {
        let now = self.clock.now_ms();
        let g = self.slots();
        let s = g.get(&folder_id)?;
        Some(match s.degraded.as_ref().filter(|d| d.until.is_none_or(|u| now < u)) {
            Some(d) => SourceHealth::Degraded {
                since: d.since,
                reason_key: d.reason_key,
            },
            None => SourceHealth::Ok {
                last_signal_at: s.last_signal_at,
            },
        })
    }
}
