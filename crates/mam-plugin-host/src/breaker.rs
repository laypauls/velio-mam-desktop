//! 시계 주입 + 크래시 차단기("10분 3회 -> 비활성"). 시각은 `Clock` 으로만 읽는다 (시험: 가짜 시계).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerConfig {
    /// 이 횟수만큼 `window` 안에 결함이 나면 비활성
    pub max_faults: usize,
    pub window: Duration,
}

impl Default for BreakerConfig {
    /// 제안값(미측정): 10분 3회
    fn default() -> Self {
        Self {
            max_faults: 3,
            window: Duration::from_secs(600),
        }
    }
}

/// 한 번 열리면(비활성) `reset` 전까지 유지된다 — 사용자가 다시 켤 때 `reset`
#[derive(Debug)]
pub struct Breaker {
    cfg: BreakerConfig,
    events: VecDeque<Instant>,
    disabled: bool,
}

impl Breaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            cfg,
            events: VecDeque::new(),
            disabled: false,
        }
    }

    /// 결함 1건 기록. 비활성이 됐으면 true
    pub fn record(&mut self, now: Instant) -> bool {
        while self
            .events
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.cfg.window)
        {
            self.events.pop_front();
        }
        self.events.push_back(now);
        if self.events.len() >= self.cfg.max_faults {
            self.disabled = true;
        }
        self.disabled
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.disabled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Time / Boundary(창 경계) / Business rule(래치)

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn t_phost_brk_01_three_faults_within_window_disable() {
        let (base, mut b) = (Instant::now(), Breaker::new(BreakerConfig::default()));
        assert!(!b.record(at(base, 0)));
        assert!(!b.record(at(base, 100)));
        assert!(b.record(at(base, 599)));
        assert!(b.is_disabled());
    }

    #[test]
    fn t_phost_brk_02_old_faults_expire_exactly_at_window() {
        let (base, mut b) = (Instant::now(), Breaker::new(BreakerConfig::default()));
        b.record(at(base, 0));
        b.record(at(base, 10));
        // 600 초 시점에 t=0 은 창 밖(>= window) 이므로 2건만 남는다
        assert!(!b.record(at(base, 600)));
        assert!(!b.is_disabled());
    }

    #[test]
    fn t_phost_brk_03_latched_until_reset_and_reset_clears_history() {
        let (base, mut b) = (Instant::now(), Breaker::new(BreakerConfig::default()));
        for s in [0, 1, 2] {
            b.record(at(base, s));
        }
        assert!(b.record(at(base, 5000)), "시간이 지나도 래치 유지");
        b.reset();
        assert!(!b.is_disabled());
        assert!(!b.record(at(base, 5001)));
    }
}
