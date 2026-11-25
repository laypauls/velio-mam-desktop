//! 기한 있는 스레드 정리 - 종료 시 한 스레드가 오래 붙잡아도 앱 종료가 끌려가지 않게 한다.

use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(5);

/// `grace` 안에 끝나는 스레드는 `join` 하고, 기한을 넘기면 나머지는 떼어 둔다(detach).
/// 떼어 둔 스레드 수를 돌려준다. `grace` 가 너무 커서 기한이 표현되지 않으면 끝까지 기다린다.
///
/// 떼어 둔 스레드는 프로세스 종료와 함께 사라진다 - 호출자는 그 스레드의 중간 상태가 크래시와 같다는 전제
/// (원자적 쓰기, 재시작 시 `running` 작업 복구)가 성립하는 경우에만 쓴다.
pub fn join_within(handles: Vec<JoinHandle<()>>, grace: Duration) -> usize {
    let deadline = Instant::now().checked_add(grace);
    let mut abandoned = 0;
    for h in handles {
        loop {
            if h.is_finished() {
                let _ = h.join();
                break;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                abandoned += 1;
                break;
            }
            std::thread::sleep(POLL);
        }
    }
    abandoned
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary (grace 0, 큰 grace) / Concurrency (오래 걸리는 스레드)
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn t_join_01_finished_threads_are_joined_and_none_abandoned() {
        // Arrange
        let handles: Vec<_> = (0..3).map(|_| std::thread::spawn(|| {})).collect();
        // Act
        let abandoned = join_within(handles, Duration::from_secs(5));
        // Assert
        assert_eq!(abandoned, 0);
    }

    #[test]
    fn t_join_02_a_busy_thread_is_abandoned_at_the_deadline() {
        // Arrange - 풀어 줄 때까지 도는 스레드
        let release = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&release);
        let busy = std::thread::spawn(move || {
            while !r.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let started = Instant::now();
        // Act
        let abandoned = join_within(vec![busy], Duration::from_millis(100));
        let took = started.elapsed();
        release.store(true, Ordering::SeqCst);
        // Assert
        assert_eq!(abandoned, 1);
        assert!(took < Duration::from_secs(2), "took {took:?}");
    }

    #[test]
    fn t_join_03_zero_grace_still_joins_an_already_finished_thread() {
        // Arrange
        let h = std::thread::spawn(|| {});
        while !h.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        // Act / Assert
        assert_eq!(join_within(vec![h], Duration::ZERO), 0);
    }

    #[test]
    fn t_join_04_huge_grace_waits_for_a_slow_thread() {
        // Arrange
        let h = std::thread::spawn(|| std::thread::sleep(Duration::from_millis(60)));
        // Act / Assert - Duration::MAX 는 기한 계산이 넘치므로 무기한 대기
        assert_eq!(join_within(vec![h], Duration::MAX), 0);
    }
}
