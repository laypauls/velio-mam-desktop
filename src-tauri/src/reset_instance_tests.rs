//! 분류: Happy / Business rule(초기화 재시작 대기) / Time / Concurrency

use std::time::{Duration, Instant};

use mam_core::protect::marker_path;
use mam_core::storage::{DataPaths, InstanceMutex, StorageError};

use super::acquire_instance_mutex_within;

fn unique(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("com.mam.test.reset.{tag}.{}.{nanos}", std::process::id())
}

fn paths_with_marker(pending: bool) -> (tempfile::TempDir, DataPaths) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = DataPaths::new(dir.path().to_path_buf());
    if pending {
        std::fs::write(marker_path(&paths), b"{}").expect("marker");
    }
    (dir, paths)
}

#[test]
fn t_reset_mutex_01_no_marker_fails_immediately() {
    // Arrange
    let (_d, paths) = paths_with_marker(false);
    let id = unique("a");
    let _held = InstanceMutex::acquire(&id).expect("held");
    // Act
    let started = Instant::now();
    let r = acquire_instance_mutex_within(&paths, &id, Duration::from_secs(5));
    // Assert
    assert!(matches!(r, Err(StorageError::AlreadyRunning)));
    assert!(started.elapsed() < Duration::from_secs(1), "must not wait");
}

#[test]
fn t_reset_mutex_02_pending_marker_waits_for_previous_process_exit() {
    // Arrange
    let (_d, paths) = paths_with_marker(true);
    let id = unique("b");
    let held = InstanceMutex::acquire(&id).expect("held");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        drop(held);
    });
    // Act
    let r = acquire_instance_mutex_within(&paths, &id, Duration::from_secs(5));
    releaser.join().expect("join");
    // Assert
    assert!(
        r.is_ok(),
        "restart must take over after the old owner exits"
    );
}

#[test]
fn t_reset_mutex_03_pending_marker_wait_is_bounded() {
    // Arrange
    let (_d, paths) = paths_with_marker(true);
    let id = unique("c");
    let _held = InstanceMutex::acquire(&id).expect("held");
    // Act
    let started = Instant::now();
    let r = acquire_instance_mutex_within(&paths, &id, Duration::from_millis(300));
    // Assert
    assert!(matches!(r, Err(StorageError::AlreadyRunning)));
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(3));
}
