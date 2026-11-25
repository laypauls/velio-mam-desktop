//! 종료 정리 실행기 — `RunEvent::Exit` 와 Windows 세션 종료(`session_end`)가 같은 정리를 공유한다.
//!
//! 정리는 한 번만 실행된다(`RunOnce`) — 세션 종료 정리 뒤에 `Exit` 가 오거나 그 반대여도 두 번 돌지 않는다.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mam_core::ai_jobs::InFlight;
use mam_core::ingest::Ingest;
use mam_plugin_runtime::{PluginJobs, PluginRuntime};
use tauri::{AppHandle, Manager};

use crate::ai::{AiRunner, AiService};
use crate::backup_scheduler::BackupScheduler;
use crate::search_service::SearchService;
use crate::storage_commands::Storage;
use crate::{player_commands, shutdown, tray};

/// 한 번만 참을 돌려주는 플래그.
#[derive(Debug, Default)]
pub struct RunOnce(AtomicBool);

impl RunOnce {
    /// 처음 부른 호출만 true.
    pub fn try_begin(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }
}

/// 정리 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 트레이 종료 등 — 진행 중인 외부 AI 요청을 최대 60초 기다린다.
    Normal,
    /// Windows 세션 종료 — OS 가 주는 시간이 짧으므로 AI 요청 대기를 생략한다 (사용량 기록 유실 가능).
    SessionEnd,
}

impl Mode {
    pub fn waits_for_ai_requests(self) -> bool {
        self == Mode::Normal
    }
}

pub struct ExitServices {
    pub progress_stop: Arc<AtomicBool>,
    pub scheduler: Arc<BackupScheduler>,
    pub search: Arc<SearchService>,
    pub ai_runner: Arc<AiRunner>,
    pub ingest: Arc<Ingest>,
    pub ai_requests: InFlight,
    pub ai_service: Arc<AiService>,
    pub plugin_jobs: Arc<PluginJobs>,
    pub plugin_runtime: Arc<PluginRuntime>,
    pub storage: Arc<Storage>,
    pub once: RunOnce,
}

impl ExitServices {
    /// 정리를 실행한다. 이미 실행됐으면 아무것도 하지 않고 false.
    /// 작업·감시 스레드를 먼저 멈추고, 큐에 남은 쓰기를 마친 뒤 쓰기 스레드를 정리한다.
    pub fn run(&self, app: &AppHandle, mode: Mode) -> bool {
        if !self.once.try_begin() {
            tracing::info!(?mode, "shutdown already ran; skipping");
            return false;
        }
        let total = std::time::Instant::now();
        self.progress_stop.store(true, Ordering::Relaxed);
        // 정지 신호를 모두 먼저 보내 각 서비스의 정리 시간이 겹치게 하고, 하나의 기한 안에서 기다린다.
        // 사이드카(FFmpeg 등)는 종료 신호로 바로 죽고, 기한을 넘긴 스레드는 기다리지 않는다 (shutdown.rs)
        shutdown::timed("signal", || {
            self.scheduler.request_stop();
            self.search.request_stop();
            self.ai_runner.request_stop();
            self.ingest.request_shutdown();
        });
        // 플레이어 워커를 먼저 종료한다 (영상 출력 창이 없으므로 UI 스레드 정리는 필요 없다)
        if let Some(player) = app.try_state::<Arc<player_commands::PlayerService>>() {
            shutdown::timed("player", || player.shutdown());
        }
        // 이미 나간 외부 AI 요청은 응답·사용량이 저장될 때까지 기다린다 (최대 60초, 새 요청은 정지 신호로 막혀 있다).
        // 이 대기는 1.2초 기한에 포함하지 않는다. 세션 종료에서는 OS 가 기다려 주지 않으므로 생략한다.
        if mode.waits_for_ai_requests() {
            shutdown::wait_ai_requests(&self.ai_requests, shutdown::AI_REQUEST_WAIT, |_| {
                tray::show_waiting_for_ai(app)
            });
        }
        // 기한을 넘긴 구독 CLI 요청과 진행 중인 브라우저 로그인 세션은 PID 로 종료한다 (앱이 끝나도 남지 않게)
        self.ai_service.terminate_cli();
        let deadline = shutdown::Deadline::new(shutdown::EXIT_GRACE);
        shutdown::timed("backup-scheduler", || {
            self.scheduler.stop_within(deadline.remaining())
        });
        // 색인 워커를 먼저 멈춘다 (DB 풀 종료 뒤에는 배치를 쓸 수 없다)
        shutdown::timed("search-worker", || {
            self.search.shutdown_within(deadline.remaining())
        });
        shutdown::timed("ai-runner", || self.ai_runner.stop_within(deadline.remaining()));
        shutdown::timed("ingest", || self.ingest.shutdown_within(deadline.remaining()));
        // 플러그인: 피드·전체 실행 스레드를 멈춘 뒤 워커 프로세스를 PID 로 종료한다 (JobRunner 는 위에서 이미 멈췄다)
        shutdown::timed("plugin-feed", || self.plugin_jobs.stop());
        shutdown::timed("plugin-workers", || self.plugin_runtime.shutdown_all());
        shutdown::timed("db-pool", || self.storage.pool.shutdown());
        tracing::info!(
            ms = total.elapsed().as_millis() as u64,
            ?mode,
            "shutdown complete"
        );
        true
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Concurrency (한 번만 실행) / Business rule (세션 종료는 AI 대기 생략)
    use super::*;

    #[test]
    fn t_sessend_04_run_once_lets_only_the_first_caller_through() {
        // Arrange
        let once = RunOnce::default();
        // Act / Assert: 세션 종료 정리 뒤에 Exit 가 와도 두 번째는 no-op
        assert!(once.try_begin());
        assert!(!once.try_begin());
        assert!(!once.try_begin());
    }

    #[test]
    fn t_sessend_04b_run_once_is_exclusive_across_threads() {
        // Arrange (Concurrency)
        let once = Arc::new(RunOnce::default());
        let wins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        // Act
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let (o, w) = (Arc::clone(&once), Arc::clone(&wins));
                std::thread::spawn(move || {
                    if o.try_begin() {
                        w.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        // Assert
        assert_eq!(wins.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn t_sessend_04c_session_end_skips_the_ai_request_wait_but_normal_exit_keeps_it() {
        // Assert
        assert!(Mode::Normal.waits_for_ai_requests());
        assert!(!Mode::SessionEnd.waits_for_ai_requests());
    }
}
