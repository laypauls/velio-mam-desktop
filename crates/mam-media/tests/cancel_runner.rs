//! 종료 신호로 사이드카를 죽이는 실행기 (`CancellableProcessRunner`).
//! 분류: Happy (신호 없으면 일반 실행) / Boundary (신호가 이미 켜짐) / Concurrency (실행 중 신호) / External (실제 프로세스)

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mam_media::MediaError;
use mam_media::tool::{CancellableProcessRunner, CommandRunner, RunOptions, Tool};

fn opts() -> RunOptions {
    RunOptions {
        timeout: Duration::from_secs(60),
        max_stdout: 1 << 20,
    }
}

#[test]
fn t_media_cancel_01_preset_flag_returns_cancelled_without_spawning() {
    // Arrange - 존재하지 않는 프로그램: 띄웠다면 Io 오류가 된다
    let runner = CancellableProcessRunner::new(Arc::new(AtomicBool::new(true)));
    // Act
    let err = runner
        .run(
            Tool::Ffmpeg,
            Path::new("Z:/no/such/ffmpeg-binary"),
            &[],
            &opts(),
        )
        .unwrap_err();
    // Assert
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
}

#[cfg(windows)]
#[test]
fn t_media_cancel_02_flag_during_run_kills_the_child_promptly() {
    // Arrange - 30초 걸리는 프로세스(ping 30회)
    let flag = Arc::new(AtomicBool::new(false));
    let runner = CancellableProcessRunner::new(Arc::clone(&flag));
    let args: Vec<OsString> = ["-n", "30", "127.0.0.1"].map(OsString::from).to_vec();
    let setter = {
        let flag = Arc::clone(&flag);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
        })
    };
    let started = Instant::now();
    // Act
    let err = runner
        .run(Tool::Ffprobe, Path::new("ping"), &args, &opts())
        .unwrap_err();
    let took = started.elapsed();
    setter.join().unwrap();
    // Assert - 시간 초과(60초)나 프로세스 종료(30초)를 기다리지 않는다
    assert!(matches!(err, MediaError::Cancelled { .. }), "{err:?}");
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[cfg(windows)]
#[test]
fn t_media_cancel_03_without_the_flag_it_behaves_like_the_plain_runner() {
    // Arrange
    let runner = CancellableProcessRunner::new(Arc::new(AtomicBool::new(false)));
    let args: Vec<OsString> = vec!["/C".into(), "exit".into(), "3".into()];
    // Act
    let out = runner
        .run(Tool::Ffmpeg, Path::new("cmd"), &args, &opts())
        .unwrap();
    // Assert
    assert_eq!(out.code, Some(3));
}
