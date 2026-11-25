//! AI 백그라운드 스레드 — 예약 루프(신규 자산·백그라운드 스윕)와 `ai://analysis` 발행. 앱 종료 시 `stop()`.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use super::{AiService, StopSignal};

/// 신규 자산 확인 주기
pub const NEW_TICK: Duration = Duration::from_secs(5);
/// 백그라운드 스윕 주기 (신규 틱 6회마다)
const SWEEP_EVERY: u32 = 6;
/// `ai://analysis` 발행 주기 (4Hz 상한)
pub const EVENT_TICK: Duration = Duration::from_millis(250);

pub struct AiRunner {
    stop: Arc<StopSignal>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl AiRunner {
    /// 시작 시 PA4 정리를 한 번 하고 두 스레드를 띄운다. 스레드를 못 만들어도 앱은 계속 동작한다(분석 자동화만 멈춘다).
    pub fn start(service: Arc<AiService>) -> Arc<Self> {
        let stop = Arc::new(StopSignal::default());
        let mut threads = Vec::new();
        {
            let (svc, stop) = (Arc::clone(&service), Arc::clone(&stop));
            threads.extend(
                std::thread::Builder::new()
                    .name("mam-ai-sched".into())
                    .spawn(move || {
                        match svc.purge_old() {
                            Ok(n) if n > 0 => {
                                tracing::info!(rows = n, "old AI usage/history purged")
                            }
                            Ok(_) => {}
                            Err(_) => tracing::warn!("cannot purge old AI usage"),
                        }
                        let mut n = 0u32;
                        loop {
                            if let Err(e) = svc.tick_new_assets() {
                                tracing::warn!(?e, "AI new-asset tick failed");
                            }
                            if n.is_multiple_of(SWEEP_EVERY) {
                                if let Err(e) = svc.tick_background() {
                                    tracing::warn!(?e, "AI background sweep failed");
                                }
                                if let Err(e) = svc.tick_embedding_backfill() {
                                    tracing::warn!(?e, "AI embedding backfill failed");
                                }
                            }
                            if let Err(e) = svc.sync_vector_indexes() {
                                tracing::warn!(?e, "vector index sync failed");
                            }
                            n = n.wrapping_add(1);
                            if stop.wait(NEW_TICK) {
                                break;
                            }
                        }
                    })
                    .map_err(|e| tracing::error!(%e, "cannot start AI scheduler thread")),
            );
        }
        {
            let (svc, stop) = (service, Arc::clone(&stop));
            threads.extend(
                std::thread::Builder::new()
                    .name("mam-ai-events".into())
                    .spawn(move || {
                        while !stop.wait(EVENT_TICK) {
                            svc.notifier.flush(svc.pool(), &*svc.sink);
                        }
                    })
                    .map_err(|e| tracing::error!(%e, "cannot start AI event thread")),
            );
        }
        Arc::new(Self {
            stop,
            threads: Mutex::new(threads),
        })
    }

    /// 종료 신호만 보낸다 (기다리지 않는다) - 다른 서비스의 정리와 시간을 겹치게 한다.
    pub fn request_stop(&self) {
        self.stop.set();
    }

    /// 스레드를 멈추고 `grace` 안에서만 기다린다 (DB 풀 종료보다 먼저). 넘긴 스레드는 떼어 둔다 (틱이 인덱스 저장 등 긴 작업 중일 수 있다). 떼어 낸 수를 돌려준다.
    pub fn stop_within(&self, grace: Duration) -> usize {
        self.stop.set();
        let handles: Vec<_> =
            std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        let abandoned = mam_core::join::join_within(handles, grace);
        if abandoned > 0 {
            tracing::warn!(abandoned, "AI threads still busy at shutdown; not waiting");
        }
        abandoned
    }
}

#[cfg(test)]
mod tests {
    // Categories: Concurrency (대기 중인 스레드를 신호로 즉시 깨움) / Boundary (두 번 정지, 기한 0)
    use std::time::Instant;

    use super::super::test_support::Fx;
    use super::*;

    #[test]
    fn t_exit_ai_01_runner_threads_wake_on_the_stop_signal_within_the_deadline() {
        // Arrange: 틱 사이 5초 대기 중인 스케줄러와 250ms 이벤트 스레드
        let fx = Fx::new();
        let runner = AiRunner::start(Arc::clone(&fx.service));
        std::thread::sleep(Duration::from_millis(150));

        // Act
        let started = Instant::now();
        runner.request_stop();
        let abandoned = runner.stop_within(Duration::from_secs(5));

        // Assert: NEW_TICK(5초)를 기다리지 않는다
        assert_eq!(abandoned, 0);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn t_exit_ai_02_stopping_twice_is_safe() {
        // Arrange
        let fx = Fx::new();
        let runner = AiRunner::start(Arc::clone(&fx.service));

        // Act
        let first = runner.stop_within(Duration::from_secs(5));
        let second = runner.stop_within(Duration::ZERO);

        // Assert: 두 번째에는 정리할 스레드가 없다
        assert_eq!(first, 0);
        assert_eq!(second, 0);
    }
}
