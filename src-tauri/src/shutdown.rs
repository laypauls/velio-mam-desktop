//! 종료 절차 — 단계별 소요 계측과 기한 (`RunEvent::Exit`).
//!
//! 종료가 느리면 어느 단계가 붙잡고 있는지 로그(`shutdown step`)만으로 알 수 있어야 한다.
//! 종료 순서: ① 모든 서비스에 정지 신호를 먼저 보내고 ② 하나의 기한 안에서 차례로 기다린 뒤 ③ DB 풀을 닫는다.
//! 기한을 넘긴 스레드(긴 인덱스 저장, 응답 없는 외부 AI 요청 등)는 기다리지 않는다 - 그 작업은 크래시와 같은 상태로
//! 남으므로 원자적 쓰기와 시작 시 복구(`running` 작업 -> `pending`)가 전제다.

use std::time::{Duration, Instant};

/// 정지 신호 이후 스레드를 기다려 주는 전체 시간. 목표 종료 시간(2초) 안에 DB 풀 정리까지 끝나도록 잡는다.
pub const EXIT_GRACE: Duration = Duration::from_millis(1_200);

/// 진행 중인 외부 AI 요청을 기다리는 최대 시간 - HTTP 요청 제한 시간 + 여유 5초.
/// 제한 시간에 끝난 요청도 실패·사용량 기록(DB 쓰기)까지 마치고 종료되도록 여유를 둔다.
pub const AI_REQUEST_WAIT: Duration =
    Duration::from_secs(mam_ai::external::REQUEST_TIMEOUT.as_secs() + 5);

/// 정지 신호를 보낸 **뒤** 호출한다. 진행 중인 외부 AI 요청이 있으면 응답·사용량이 저장될 때까지(최대 `cap`) 기다린다.
/// 이미 나간 요청을 끊으면 다음 실행이 같은 요청을 다시 보내 과금만 두 번 된다. 정지 신호 이후에는 새 요청이 시작되지 않으므로
/// 기다리는 대상은 늘지 않는다. 요청이 없으면 즉시 돌아온다. 기한 안에 끝났으면 true.
/// `on_wait` 는 실제로 기다리게 될 때 한 번 호출된다 (트레이 안내). 기다린 시간은 `EXIT_GRACE` 에 포함되지 않는다.
pub fn wait_ai_requests(
    in_flight: &mam_core::ai_jobs::InFlight,
    cap: Duration,
    on_wait: impl FnOnce(usize),
) -> bool {
    let started = Instant::now();
    let idle = in_flight.wait_idle(cap, |n| {
        tracing::info!(requests = n, "waiting for in-flight external AI requests");
        on_wait(n);
    });
    if !idle {
        tracing::warn!(
            requests = in_flight.count(),
            "external AI request still running at the wait cap; not waiting"
        );
    }
    tracing::info!(
        ms = started.elapsed().as_millis() as u64,
        idle,
        "shutdown ai-requests wait"
    );
    idle
}

/// `f` 를 실행하고 걸린 시간을 `shutdown step` 으로 기록한다.
pub fn timed<T>(step: &'static str, f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let out = f();
    tracing::info!(
        step,
        ms = started.elapsed().as_millis() as u64,
        "shutdown step"
    );
    out
}

/// 여러 단계가 함께 쓰는 기한. 앞 단계가 시간을 쓰면 뒤 단계가 기다릴 수 있는 시간이 줄어든다.
#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    started: Instant,
    grace: Duration,
}

impl Deadline {
    pub fn new(grace: Duration) -> Self {
        Self {
            started: Instant::now(),
            grace,
        }
    }

    /// 남은 시간 (기한이 지났으면 0).
    pub fn remaining(&self) -> Duration {
        self.grace.saturating_sub(self.started.elapsed())
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary (기한 소진)
    use super::*;

    #[test]
    fn t_exit_01_deadline_counts_down_and_saturates_at_zero() {
        // Arrange
        let d = Deadline::new(Duration::from_millis(30));
        let first = d.remaining();
        // Act
        std::thread::sleep(Duration::from_millis(60));
        // Assert
        assert!(first <= Duration::from_millis(30));
        assert_eq!(d.remaining(), Duration::ZERO);
    }

    #[test]
    fn t_exit_02_timed_returns_the_value_of_the_step() {
        // Arrange / Act
        let v = timed("test-step", || 7);
        // Assert
        assert_eq!(v, 7);
    }

    #[test]
    fn t_exit_03_ai_wait_is_immediate_when_no_request_is_in_flight() {
        // Arrange (Boundary): 요청 없음
        let f = mam_core::ai_jobs::InFlight::new();
        let mut notified = false;
        let started = Instant::now();
        // Act
        let idle = wait_ai_requests(&f, AI_REQUEST_WAIT, |_| notified = true);
        // Assert: 종료 시간은 늘지 않고 트레이 안내도 없다
        assert!(idle && !notified);
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn t_exit_04_ai_wait_cap_is_the_http_timeout_and_is_honored() {
        // Arrange (Time): 놓이지 않는 요청 + 짧은 상한을 주입
        assert_eq!(
            AI_REQUEST_WAIT,
            mam_ai::external::REQUEST_TIMEOUT + Duration::from_secs(5)
        );
        let f = mam_core::ai_jobs::InFlight::new();
        let _g = f.begin();
        let mut notified = 0;
        let started = Instant::now();
        // Act
        let idle = wait_ai_requests(&f, Duration::from_millis(100), |n| notified = n);
        // Assert: 상한에서 포기하고(요청은 떼어 둔다), 대기 안내는 한 번 나갔다
        assert!(!idle);
        assert_eq!(notified, 1);
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn t_exit_05_ai_wait_returns_as_soon_as_the_request_finishes() {
        // Arrange (Concurrency)
        let f = mam_core::ai_jobs::InFlight::new();
        let g = f.begin();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            drop(g);
        });
        let started = Instant::now();
        // Act
        let idle = wait_ai_requests(&f, Duration::from_secs(30), |_| {});
        t.join().unwrap();
        // Assert: 상한(30초)까지 가지 않는다
        assert!(idle);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
