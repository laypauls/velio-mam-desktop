//! 앱 종료 중의 처리 작업 (종료가 느려지던 문제의 회귀: 사이드카가 끝나길 기다리며 종료가 10초 넘게 걸렸다).
//!
//! 분류: Concurrency (실행 중 종료 신호) / Business rule (실패·시도 횟수로 세지 않음, 다음 실행에서 이어짐) /
//! Boundary (기한 0·기한 초과) / External (응답 없는 사이드카).
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기. ffmpeg 는 대역 실행기로 "오래 걸리는 프로세스"를 흉내 낸다.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::env::{Env, wait_until};
use common::proc_env::{ProcEnv, StubRunner, ok_output, probe_json};
use mam_core::jobs::JobQueue;
use mam_media::MediaError;
use mam_media::tool::{MediaTools, Tool};

/// ffprobe 는 정상. ffmpeg 는 `stop` 이 켜질 때까지(최대 20초) 돈다 - 켜지면 실제 실행기처럼 `Cancelled`.
/// `cooperative` 가 false 면 `stop` 을 무시하고 `release` 까지 돈다 (응답 없는 사이드카).
fn blocking_tools(
    stop: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
    cooperative: bool,
) -> Arc<MediaTools> {
    let runner = StubRunner(Box::new(move |tool, _| match tool {
        Tool::Ffprobe => ok_output(probe_json()),
        Tool::Ffmpeg => {
            entered.store(true, Ordering::SeqCst);
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(20) {
                if cooperative && stop.load(Ordering::SeqCst) {
                    return Err(MediaError::Cancelled { tool: "ffmpeg" });
                }
                if !cooperative && release.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(MediaError::Timeout {
                tool: "ffmpeg",
                timeout: Duration::from_secs(20),
            })
        }
    }));
    Arc::new(MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(runner),
    ))
}

fn thumb_job(p: &ProcEnv) -> Option<(String, i64)> {
    p.env.read(|c| {
        c.query_row(
            "SELECT status, attempts FROM jobs WHERE kind = 'thumb'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    })
}

/// 영상 1개를 등록하고 ffmpeg 대역이 호출될 때까지 기다린다 (썸네일 작업이 사이드카 안에 있는 상태).
fn start_with_video_in_flight(p: &ProcEnv, entered: &AtomicBool) {
    p.put("lib/v.mp4", b"stubbed video");
    p.env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap();
    p.env.ingest.start().unwrap();
    assert!(
        wait_until(Duration::from_secs(30), || entered.load(Ordering::SeqCst)),
        "ffmpeg stub was never called:\n{}",
        p.jobs_dump()
    );
}

#[test]
fn t_proc_sd_01_shutdown_kills_the_inflight_sidecar_and_leaves_the_job_resumable() {
    // Arrange: 썸네일 작업이 오래 걸리는 ffmpeg 안에 있다
    let (_unused, release, entered) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let env = Env::new();
    // 실제 배선과 같은 신호: 사이드카 실행기는 작업 대기열의 종료 핸들을 본다
    let handle = env.ingest.jobs().shutdown_handle();
    let tools = blocking_tools(
        Arc::clone(&handle),
        Arc::clone(&release),
        Arc::clone(&entered),
        true,
    );
    let p = ProcEnv::with_env(env, tools);
    start_with_video_in_flight(&p, &entered);

    // Act
    let started = Instant::now();
    let abandoned = p.env.ingest.shutdown_within(Duration::from_secs(5));
    let took = started.elapsed();

    // Assert: 사이드카 시간 초과(20초)를 기다리지 않고 기한 안에 모두 정리된다
    assert_eq!(abandoned, 0);
    assert!(took < Duration::from_secs(3), "shutdown took {took:?}");
    // 작업은 실패·시도 횟수로 세지 않고 대기로 돌아가 다음 실행에서 이어진다
    assert_eq!(thumb_job(&p), Some(("pending".to_string(), 0)));
    let failed = p
        .env
        .count("SELECT COUNT(*) FROM asset_derivatives WHERE state = 'failed'");
    assert_eq!(
        failed, 0,
        "an interrupted sidecar must not be recorded as failed"
    );
}

#[test]
fn t_proc_sd_02_unresponsive_sidecar_is_abandoned_at_the_deadline_and_recovered_next_start() {
    // Arrange: 종료 신호를 무시하는 ffmpeg (응답 없는 사이드카)
    let (stop, release, entered) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let tools = blocking_tools(stop, Arc::clone(&release), Arc::clone(&entered), false);
    let p = ProcEnv::new(tools);
    start_with_video_in_flight(&p, &entered);

    // Act
    let started = Instant::now();
    let abandoned = p.env.ingest.shutdown_within(Duration::from_millis(300));
    let took = started.elapsed();

    // Assert: 기한(300ms) 근처에서 돌아오고 멈추지 않은 스레드를 센다
    assert!(abandoned >= 1, "abandoned = {abandoned}");
    assert!(took < Duration::from_secs(3), "shutdown took {took:?}");
    // 작업 행은 running 으로 남는다 -> "재시작" 의 recover 가 시도 횟수 변화 없이 pending 으로 되돌린다
    assert_eq!(thumb_job(&p), Some(("running".to_string(), 0)));
    let next_start = JobQueue::new(Arc::clone(&p.env.pool), p.env.clock.clone());
    assert!(next_start.recover().unwrap() >= 1);
    assert_eq!(thumb_job(&p), Some(("pending".to_string(), 0)));
    release.store(true, Ordering::SeqCst);
}

#[test]
fn t_proc_sd_03_zero_grace_returns_immediately_even_with_a_busy_worker() {
    // Arrange
    let (stop, release, entered) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let tools = blocking_tools(stop, Arc::clone(&release), Arc::clone(&entered), false);
    let p = ProcEnv::new(tools);
    start_with_video_in_flight(&p, &entered);

    // Act
    let started = Instant::now();
    let abandoned = p.env.ingest.shutdown_within(Duration::ZERO);

    // Assert
    assert!(abandoned >= 1);
    assert!(started.elapsed() < Duration::from_secs(2));
    release.store(true, Ordering::SeqCst);
}

#[test]
fn t_proc_sd_04_shutdown_handle_follows_request_shutdown_and_is_idempotent() {
    // Arrange
    let env = Env::new();
    let handle = env.ingest.jobs().shutdown_handle();
    assert!(!handle.load(Ordering::SeqCst));

    // Act: 시작 전에도, 두 번 불러도 안전하다
    env.ingest.request_shutdown();
    env.ingest.request_shutdown();

    // Assert
    assert!(handle.load(Ordering::SeqCst));
    assert!(env.ingest.jobs().is_shutdown());
    assert_eq!(env.ingest.shutdown_within(Duration::from_secs(1)), 0);
}
