//! 사이드카 일시 오류(부하 시 시간 초과)와 영구 오류의 구분 (P2-3 후속, 부하 의존 간헐 실패 수정).
//!
//! 분류: External (14, 15) / Business rule (14, 15) / Boundary (16: 재시도 소진).
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기. ffmpeg/ffprobe 는 대역 실행기로 호출 횟수별 결과를 정한다.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::env::T0;
use common::proc_env::{ProcEnv, StubRunner, ok_output, png_bytes, probe_json};
use mam_media::MediaError;
use mam_media::tool::{MediaTools, Tool};

/// ffprobe 는 정상, ffmpeg 는 호출 순번(0부터)에 따라 `plan` 이 결과를 정한다.
fn tools_with_ffmpeg_plan(
    calls: Arc<AtomicUsize>,
    plan: impl Fn(usize) -> Result<(), MediaError> + Send + Sync + 'static,
) -> Arc<MediaTools> {
    let runner = StubRunner(Box::new(move |tool, _| match tool {
        Tool::Ffprobe => ok_output(probe_json()),
        Tool::Ffmpeg => {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            plan(n)?;
            ok_output(png_bytes(64, 36))
        }
    }));
    Arc::new(MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(runner),
    ))
}

fn timeout() -> MediaError {
    MediaError::Timeout {
        tool: "ffmpeg",
        timeout: std::time::Duration::from_secs(30),
    }
}

fn add_video_and_run(p: &ProcEnv) -> i64 {
    p.put("lib/v.mp4", b"stubbed video");
    p.env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap();
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle(), "jobs did not drain:\n{}", p.jobs_dump());
    p.shutdown();
    p.asset_of("v.mp4")
}

#[test]
fn t_proc_14_sidecar_timeouts_are_retried_with_backoff_and_end_done() {
    // Arrange: ffmpeg 첫 두 호출이 시간 초과 (부하로 느려진 상황), 이후 정상
    let calls = Arc::new(AtomicUsize::new(0));
    let p = ProcEnv::new(tools_with_ffmpeg_plan(Arc::clone(&calls), |n| {
        if n < 2 { Err(timeout()) } else { Ok(()) }
    }));

    // Act
    let vid = add_video_and_run(&p);

    // Assert: 영구 failed 로 굳지 않고 재시도 끝에 done (스트립 포함), 백오프만큼 시계가 당겨졌다
    let thumb = p.deriv(vid, "thumb");
    assert_eq!(
        thumb.as_ref().map(|t| t.0.as_str()),
        Some("done"),
        "{thumb:?}"
    );
    let strip = p.deriv(vid, "strip");
    assert_eq!(
        strip.as_ref().map(|t| t.0.as_str()),
        Some("done"),
        "{strip:?}"
    );
    assert!(calls.load(Ordering::SeqCst) >= 3);
    assert!(p.env.clock_now() > T0, "retry must go through the backoff");
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_proc_15_decode_errors_stay_permanent_and_are_not_retried() {
    // Arrange: ffmpeg 가 디코더 오류로 종료 (입력이 손상됨 - 반복해도 소용없다)
    let calls = Arc::new(AtomicUsize::new(0));
    let p = ProcEnv::new(tools_with_ffmpeg_plan(Arc::clone(&calls), |_| {
        Err(MediaError::ToolFailed {
            tool: "ffmpeg",
            code: Some(1),
            stderr: "Invalid data found when processing input".into(),
        })
    }));

    // Act
    let vid = add_video_and_run(&p);

    // Assert: 곧바로 failed + 오류 기록, 백오프(시계 이동) 없음
    let (state, err) = p.deriv(vid, "thumb").expect("thumb state");
    assert_eq!(state, "failed");
    assert!(err.unwrap_or_default().contains("Invalid data"));
    assert_eq!(p.env.clock_now(), T0, "permanent errors must not back off");
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_proc_16_persistent_timeouts_end_failed_after_max_attempts() {
    // Arrange: ffmpeg 가 항상 시간 초과
    let calls = Arc::new(AtomicUsize::new(0));
    let p = ProcEnv::new(tools_with_ffmpeg_plan(Arc::clone(&calls), |_| {
        Err(timeout())
    }));

    // Act
    let vid = add_video_and_run(&p);

    // Assert: 무한 재시도하지 않고 마지막 시도에서 failed 로 기록
    let (state, err) = p.deriv(vid, "thumb").expect("thumb state");
    assert_eq!(state, "failed");
    assert!(err.unwrap_or_default().contains("timed out"));
    assert!(calls.load(Ordering::SeqCst) >= mam_core::jobs::MAX_ATTEMPTS as usize);
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
}
