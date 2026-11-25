//! CPU 시간 계량 — interrupt handler 가 읽는다. 호출 시작부터의 경과 시간에서 호스트 함수 대기 시간을 뺀 값이 한도를 넘으면 중단.
//! (대기 중에는 JS 가 돌지 않으므로 handler 도 불리지 않는다 — 그 구간의 상한은 호스트의 호출 타임아웃이다)

use std::cell::Cell;
use std::time::{Duration, Instant};

pub struct Meter {
    limit: Duration,
    start: Cell<Instant>,
    blocked: Cell<Duration>,
    tripped: Cell<bool>,
}

impl Meter {
    /// `cpu_ms == 0` 이면 제한 없음
    pub fn new(cpu_ms: u64) -> Self {
        Self {
            limit: Duration::from_millis(cpu_ms),
            start: Cell::new(Instant::now()),
            blocked: Cell::new(Duration::ZERO),
            tripped: Cell::new(false),
        }
    }

    pub fn begin(&self) {
        self.start.set(Instant::now());
        self.blocked.set(Duration::ZERO);
        self.tripped.set(false);
    }

    pub fn add_blocked(&self, d: Duration) {
        self.blocked.set(self.blocked.get() + d);
    }

    /// interrupt handler 용 — 한도를 넘었으면 true 를 돌려주고 기록한다
    pub fn check(&self) -> bool {
        if self.limit.is_zero() {
            return false;
        }
        let used = self
            .start
            .get()
            .elapsed()
            .saturating_sub(self.blocked.get());
        if used > self.limit {
            self.tripped.set(true);
        }
        self.tripped.get()
    }

    pub fn tripped(&self) -> bool {
        self.tripped.get()
    }
}
