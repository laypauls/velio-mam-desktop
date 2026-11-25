//! 호스트 활동 계수기 — 시험이 고정 sleep 없이 "호출이 호스트에 들어갔다/종료 요청이 도착했다"를 신호로 기다리게 한다.
//! 동작에는 영향이 없다(카운터 증가 + 알림뿐). 조건 변수로만 기다리므로 폴링이 없다.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostActivity {
    /// 감독자 락을 쥐고 호출 처리에 들어간 횟수
    pub invokes_started: u64,
    /// `shutdown()` 이 호출된 횟수 (락을 기다리기 **전에** 센다)
    pub shutdowns_requested: u64,
}

#[derive(Default)]
pub(crate) struct ActivityLog {
    cur: Mutex<HostActivity>,
    cv: Condvar,
}

impl ActivityLog {
    fn guard(&self) -> MutexGuard<'_, HostActivity> {
        self.cur.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn bump(&self, f: impl FnOnce(&mut HostActivity)) {
        f(&mut self.guard());
        self.cv.notify_all();
    }

    pub(crate) fn snapshot(&self) -> HostActivity {
        *self.guard()
    }

    /// 조건이 참이 될 때까지 조건 변수로 기다린다. 제한 시간 안에 안 되면 false
    pub(crate) fn wait_until(
        &self,
        timeout: Duration,
        pred: impl Fn(&HostActivity) -> bool,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = self.guard();
        while !pred(&g) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            g = self
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        true
    }
}
