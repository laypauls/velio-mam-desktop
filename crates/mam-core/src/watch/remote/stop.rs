//! 중지 신호 + 중단 가능한 대기 (조건 변수 — 폴링 루프 없음).

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Default)]
pub(super) struct StopSignal {
    /// (중지 요청, 깨우기 요청 — 설정 변경으로 접속을 다시 맺어야 함)
    flags: Mutex<(bool, bool)>,
    cv: Condvar,
}

/// [`StopSignal::wait`] 가 깨어난 이유
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Woke {
    Timeout,
    Stop,
    /// 접속 설정이 바뀌었다 — 접속을 닫고 설정을 다시 읽는다
    Reload,
}

impl StopSignal {
    fn flags(&self) -> MutexGuard<'_, (bool, bool)> {
        self.flags.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(super) fn stop(&self) {
        self.flags().0 = true;
        self.cv.notify_all();
    }

    pub(super) fn reload(&self) {
        self.flags().1 = true;
        self.cv.notify_all();
    }

    pub(super) fn is_stopped(&self) -> bool {
        self.flags().0
    }

    /// 읽고 지운다
    pub(super) fn take_reload(&self) -> bool {
        std::mem::take(&mut self.flags().1)
    }

    /// 최대 `d` 를 기다린다. 중지·재설정 요청이 오면 즉시 깨어난다.
    pub(super) fn wait(&self, d: Duration) -> Woke {
        let g = self.flags();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, d, |f| !f.0 && !f.1)
            .unwrap_or_else(|p| p.into_inner());
        if g.0 {
            Woke::Stop
        } else if g.1 {
            Woke::Reload
        } else {
            Woke::Timeout
        }
    }
}
