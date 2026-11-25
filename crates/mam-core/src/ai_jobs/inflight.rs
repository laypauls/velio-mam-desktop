//! 진행 중인 외부 AI 요청 수 - 종료 경로가 "응답이 저장될 때까지" 기다릴 수 있게 한다.
//!
//! 요청이 이미 나갔다면 비용은 발생했다. 응답·사용량이 커밋되기 전에 프로세스가 끝나면 작업은 재시작 시 복구되어
//! 같은 요청을 다시 보낸다(과금 2회·기록 1회). 그래서 [`InFlightGuard`] 는 요청 전에 잡고 **결과가 SQLite 에 커밋된 뒤**
//! (또는 오류가 처리된 뒤) 놓는다.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Inner {
    count: Mutex<usize>,
    idle: Condvar,
}

/// 진행 중인 외부 요청 계수기 (복제하면 같은 계수기를 가리킨다).
#[derive(Clone, Default)]
pub struct InFlight(Arc<Inner>);

impl InFlight {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, usize> {
        self.0.count.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 지금 진행 중인 요청 수 (잠금 한 번, 저렴하다).
    pub fn count(&self) -> usize {
        *self.lock()
    }

    /// 요청 하나를 등록한다. 반환된 가드가 drop(오류·패닉 포함)될 때 줄어든다.
    pub fn begin(&self) -> InFlightGuard {
        *self.lock() += 1;
        InFlightGuard(self.clone())
    }

    /// 진행 중인 요청이 0 이 될 때까지 최대 `cap` 기다린다. 0 에 도달했으면 true.
    /// `on_wait` 는 기다려야 할 때(처음 한 번만) 호출된다 - 트레이 안내 등.
    pub fn wait_idle(&self, cap: Duration, on_wait: impl FnOnce(usize)) -> bool {
        let mut n = self.lock();
        if *n == 0 {
            return true;
        }
        on_wait(*n);
        let Some(deadline) = Instant::now().checked_add(cap) else {
            while *n > 0 {
                n = self.0.idle.wait(n).unwrap_or_else(|e| e.into_inner());
            }
            return true;
        };
        while *n > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            n = self
                .0
                .idle
                .wait_timeout(n, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }
}

/// [`InFlight::begin`] 이 돌려주는 RAII 가드.
pub struct InFlightGuard(InFlight);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let mut n = self.0.lock();
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.0.0.idle.notify_all();
        }
    }
}

#[cfg(test)]
#[path = "inflight_tests.rs"]
mod tests;
