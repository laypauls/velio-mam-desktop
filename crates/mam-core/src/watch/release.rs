//! 핸들 해제 추적 (ADR-1) — 감시를 멈춘 뒤 OS 디렉터리 핸들이 실제로 닫힐 때까지 기다리는 데 쓴다.
//!
//! 감시를 시작할 때마다 `SinkId` 를 열고, 어댑터는 OS 핸들을 쥔 동안 `ReleaseToken` 을 쥔다(여러 개 가능).
//! `stop` 은 sink 를 닫고, `wait_released` 는 **닫힌 sink 의 토큰이 모두 버려질 때까지** 기다린다.
//! 토큰이 없는 어댑터(`scan_only`)는 기다릴 것이 없다 — 감시 엔트리 수와 비교하던 옛 방식은 이 경우 거짓 "해제됨" 을 냈다.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SinkId(u64);

#[derive(Default)]
struct TrackerState {
    next: u64,
    /// 살아 있는 토큰 수 (0 이면 항목을 지운다)
    live: HashMap<SinkId, usize>,
    open: HashSet<SinkId>,
}

#[derive(Default)]
pub(crate) struct ReleaseTracker {
    state: Mutex<TrackerState>,
    cv: Condvar,
}

impl ReleaseTracker {
    fn lock(&self) -> MutexGuard<'_, TrackerState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn open(&self) -> SinkId {
        let mut s = self.lock();
        s.next += 1;
        let id = SinkId(s.next);
        s.open.insert(id);
        id
    }

    /// sink 를 닫는다 (더 이상 감시 중이 아님). 토큰이 남아 있으면 `wait_released` 가 기다린다.
    pub(crate) fn close(&self, id: SinkId) {
        self.lock().open.remove(&id);
        self.cv.notify_all();
    }

    pub(crate) fn token(self: &Arc<Self>, id: SinkId) -> ReleaseToken {
        *self.lock().live.entry(id).or_insert(0) += 1;
        ReleaseToken {
            tracker: Arc::clone(self),
            id,
        }
    }

    /// 닫힌 sink 의 토큰이 모두 버려졌으면 true, 시간 초과면 false. 열린(감시 중인) sink 는 기다리지 않는다.
    pub(crate) fn wait_released(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut s = self.lock();
        loop {
            if s.live.keys().all(|id| s.open.contains(id)) {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            s = self
                .cv
                .wait_timeout(s, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    #[cfg(test)]
    pub(crate) fn live_tokens(&self) -> usize {
        self.lock().live.values().sum()
    }
}

/// OS 핸들이 열려 있을 수 있는 동안 쥔다. 버리면 핸들이 닫힌 것이다.
pub struct ReleaseToken {
    tracker: Arc<ReleaseTracker>,
    id: SinkId,
}

impl Drop for ReleaseToken {
    fn drop(&mut self) {
        let mut s = self.tracker.lock();
        if let Some(n) = s.live.get_mut(&self.id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                s.live.remove(&self.id);
            }
        }
        drop(s);
        self.tracker.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Business rule (닫힌 sink 만 기다림) / Boundary (토큰 0개) / Concurrency (다른 스레드에서 해제)
    use super::*;

    #[test]
    fn t_wmgr_rel_01_closed_sink_with_token_blocks_until_dropped() {
        // Arrange
        let t = Arc::new(ReleaseTracker::default());
        let id = t.open();
        let token = t.token(id);
        t.close(id);

        // Act + Assert
        assert!(!t.wait_released(Duration::from_millis(30)));
        drop(token);
        assert!(t.wait_released(Duration::from_millis(30)));
        assert_eq!(t.live_tokens(), 0);
    }

    #[test]
    fn t_wmgr_rel_02_open_sink_and_tokenless_sink_do_not_block() {
        // Arrange
        let t = Arc::new(ReleaseTracker::default());
        let watching = t.open();
        let _held = t.token(watching);
        let tokenless = t.open();

        // Act
        t.close(tokenless);

        // Assert
        assert!(t.wait_released(Duration::from_millis(10)));
    }

    #[test]
    fn t_wmgr_rel_03_release_from_another_thread_wakes_the_waiter() {
        // Arrange
        let t = Arc::new(ReleaseTracker::default());
        let id = t.open();
        let token = t.token(id);
        t.close(id);

        // Act
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            drop(token);
        });

        // Assert
        assert!(t.wait_released(Duration::from_secs(5)));
        h.join().unwrap();
    }
}
