//! 종료 신호 + 점검 대기 — 스레드 sleep 을 종료·장치 변경 알림으로 즉시 깨운다.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// [`StopFlag::wait_tick`] 이 돌아온 이유
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tick {
    Stopped,
    /// 장치 변경 알림 (디바운스 끝)
    Woken,
    Elapsed,
}

#[derive(Default)]
struct State {
    stopped: bool,
    /// 장치 변경 알림 뒤 점검을 시작할 시각 (알림이 이어지면 뒤로 밀린다)
    wake_at: Option<Instant>,
}

#[derive(Default)]
pub(super) struct StopFlag {
    state: Mutex<State>,
    cv: Condvar,
}

impl StopFlag {
    fn guard(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(super) fn set(&self) {
        self.guard().stopped = true;
        self.cv.notify_all();
    }

    pub(super) fn is_set(&self) -> bool {
        self.guard().stopped
    }

    /// 점검 깨우기 요청 — `debounce` 동안 조용해지면 [`Self::wait_tick`] 이 `Woken` 으로 돌아온다.
    /// 짧은 시간에 여러 번(파티션 여러 개 동시 연결) 불려도 점검은 한 번이다.
    pub(super) fn request_wake(&self, debounce: Duration) {
        self.guard().wake_at = Some(Instant::now() + debounce);
        self.cv.notify_all();
    }

    /// 최대 `timeout` 동안 기다린다. 종료 신호나 시간 경과 중 먼저 오는 쪽으로 돌아오고, 깨우기 요청은 **소비하지 않는다**.
    /// 점검(`wait_tick`) 말고 같은 플래그를 쓰는 다른 스레드(변경 대기열 처리기)용 — 깨우기 요청이 점검 대신 이 스레드에 잡히면 점검이 깨지 않는다.
    pub(super) fn wait_stop(&self, timeout: Duration) -> Tick {
        let deadline = Instant::now() + timeout;
        let mut g = self.guard();
        loop {
            if g.stopped {
                return Tick::Stopped;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Tick::Elapsed;
            }
            g = self.cv.wait_timeout(g, left).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// 최대 `timeout` 동안 기다린다. 종료 신호·디바운스가 끝난 깨우기 요청·시간 경과 중 먼저 오는 쪽으로 돌아온다.
    pub(super) fn wait_tick(&self, timeout: Duration) -> Tick {
        let deadline = Instant::now() + timeout;
        let mut g = self.guard();
        loop {
            if g.stopped {
                return Tick::Stopped;
            }
            let now = Instant::now();
            if g.wake_at.is_some_and(|w| now >= w) {
                g.wake_at = None;
                return Tick::Woken;
            }
            if now >= deadline {
                return Tick::Elapsed;
            }
            let until = g.wake_at.map_or(deadline, |w| w.min(deadline));
            g = self
                .cv
                .wait_timeout(g, until.saturating_duration_since(now))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (깨우기·경과) / Boundary (디바운스 묶음, 종료 우선) / Concurrency (다른 스레드의 알림)
    use std::sync::Arc;

    use super::*;

    const LONG: Duration = Duration::from_secs(30);

    #[test]
    fn t_devwake_01_wake_request_interrupts_the_long_wait_after_the_debounce() {
        // Arrange
        let f = Arc::new(StopFlag::default());
        let f2 = Arc::clone(&f);
        let t = std::thread::spawn(move || (f2.wait_tick(LONG), Instant::now()));

        // Act: 기준점은 요청 직전에 메인 스레드에서 잰다 (스레드 시작 시각에 의존하지 않는다)
        let requested_at = Instant::now();
        f.request_wake(Duration::from_millis(60));
        let (tick, woken_at) = t.join().unwrap();
        let waited = woken_at.duration_since(requested_at);

        // Assert: 30초를 기다리지 않고 디바운스 뒤에 깨어난다
        assert_eq!(tick, Tick::Woken);
        assert!(waited >= Duration::from_millis(50), "debounce honoured: {waited:?}");
        assert!(waited < Duration::from_secs(10), "did not wait the full tick: {waited:?}");
    }

    #[test]
    fn t_devwake_02_burst_of_requests_collapses_into_one_wake() {
        // Arrange: 디바운스 안에 3번 요청
        let f = StopFlag::default();
        for _ in 0..3 {
            f.request_wake(Duration::from_millis(40));
        }

        // Act / Assert: 한 번 깨어나고, 이어서는 깨우기 없이 시간 경과
        assert_eq!(f.wait_tick(LONG), Tick::Woken);
        assert_eq!(f.wait_tick(Duration::from_millis(80)), Tick::Elapsed);
    }

    #[test]
    fn t_devwake_03_repeated_requests_push_the_wake_back_but_never_lose_it() {
        // Arrange
        let f = StopFlag::default();
        f.request_wake(Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(10));
        let requested_at = Instant::now(); // 두 번째 요청 직전 — 요청 뒤에 재면 부하 지연만큼 대기가 짧게 보인다
        f.request_wake(Duration::from_millis(80)); // 뒤로 밀림

        // Act
        let tick = f.wait_tick(LONG);

        // Assert: 첫 요청의 30ms 가 아니라 뒤로 밀린 시각 이후
        assert_eq!(tick, Tick::Woken);
        assert!(requested_at.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn t_devwake_04_stop_wins_over_pending_wake_and_wakes_waiters() {
        // Arrange
        let f = Arc::new(StopFlag::default());
        f.request_wake(Duration::ZERO);
        f.set();

        // Act / Assert: 종료가 먼저
        assert_eq!(f.wait_tick(LONG), Tick::Stopped);
        assert!(f.is_set());
        // 대기 중인 스레드도 종료로 깨어난다
        let f = Arc::new(StopFlag::default());
        let f2 = Arc::clone(&f);
        let t = std::thread::spawn(move || f2.wait_tick(LONG));
        f.set();
        assert_eq!(t.join().unwrap(), Tick::Stopped);
    }

    #[test]
    fn t_devwake_06_wait_stop_ignores_wake_requests_so_the_supervisor_still_gets_them() {
        // Arrange: 다른 스레드(처리기)가 wait_stop 으로 기다리는 중에 깨우기 요청이 온다
        let f = Arc::new(StopFlag::default());
        let f2 = Arc::clone(&f);
        let other = std::thread::spawn(move || f2.wait_stop(Duration::from_millis(300)));

        // Act
        f.request_wake(Duration::from_millis(20));

        // Assert: 그 스레드는 요청을 먹지 않고 시간 경과로 끝나며, 점검은 여전히 Woken 을 받는다
        assert_eq!(other.join().unwrap(), Tick::Elapsed);
        assert_eq!(f.wait_tick(LONG), Tick::Woken);
        // 종료 신호에는 깨어난다
        f.set();
        assert_eq!(f.wait_stop(LONG), Tick::Stopped);
    }

    #[test]
    fn t_devwake_05_without_requests_the_tick_elapses() {
        let f = StopFlag::default();

        assert_eq!(f.wait_tick(Duration::from_millis(30)), Tick::Elapsed);
        assert_eq!(f.wait_tick(Duration::ZERO), Tick::Elapsed);
    }
}
