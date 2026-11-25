//! 폴링 스레드 중지 신호 — 기다리는 중에도 바로 깨운다.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Default)]
pub struct StopSignal {
    stopped: Mutex<bool>,
    cv: Condvar,
}

impl StopSignal {
    fn lock(&self) -> MutexGuard<'_, bool> {
        self.stopped.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set(&self) {
        *self.lock() = true;
        self.cv.notify_all();
    }

    pub fn is_set(&self) -> bool {
        *self.lock()
    }

    /// `dur` 동안 기다린다. 중지 요청이 오면 즉시 true.
    pub fn wait(&self, dur: Duration) -> bool {
        let g = self.lock();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, dur, |stopped| !*stopped)
            .unwrap_or_else(|p| p.into_inner());
        *g
    }
}

/// 한 번 열리면 계속 열려 있는 문. 기준선을 처음 시도했는지 알리는 데 쓴다.
#[derive(Default)]
pub struct Latch {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Latch {
    pub fn open(&self) {
        *self.open.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cv.notify_all();
    }

    /// 열릴 때까지 최대 `dur` 기다린다. 열렸으면 true.
    pub fn wait(&self, dur: Duration) -> bool {
        let g = self.open.lock().unwrap_or_else(|p| p.into_inner());
        let (g, _) = self
            .cv
            .wait_timeout_while(g, dur, |open| !*open)
            .unwrap_or_else(|p| p.into_inner());
        *g
    }
}
