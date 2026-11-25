//! 세션 종료(Windows 종료·로그오프)의 짧은 기한 정리가 표식을 지우는지 (T-sessend-05).
//!
//! 분류: Happy (짧은 기한 안에 끝난 정리는 표식을 지운다). 기한을 넘긴 정리는 표식을 남긴다 —
//! `watch_pending_options.rs` T-pending-34 가 이미 다룬다(Concurrency/External).
//! 실제 SQLite + 임시 폴더 + 실제 스레드.
#![cfg(windows)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::env::Env;
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity};

fn marker_rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM settings WHERE key = 'ingest_run_marker'")
}

fn new_ingest(env: &Env) -> Arc<Ingest> {
    let clock: SharedClock = env.clock.clone();
    Ingest::new(
        IngestDeps {
            pool: Arc::clone(&env.pool),
            clock,
            volumes: env.volumes.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![env.data.clone()],
            ..IngestConfig::default()
        },
    )
}

#[test]
fn t_sessend_05_shutdown_within_a_short_deadline_clears_the_marker_so_the_next_start_is_clean() {
    // Arrange: 실행 중인 서비스 + 실시간 감시 폴더 1개
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.ingest.start().unwrap();
    env.ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap();
    assert_eq!(marker_rows(&env), 1, "a running service holds the marker");

    // Act: 세션 종료 경로와 같은 기한(2초)으로 종료
    let abandoned = env.ingest.shutdown_within(Duration::from_secs(2));

    // Assert: 떼어 낸 스레드 없이 표식이 지워지고, 다음 시작은 비정상 종료로 보지 않는다
    assert_eq!(abandoned, 0);
    assert_eq!(marker_rows(&env), 0);
    assert!(new_ingest(&env).begin_run().unwrap().is_empty());
}
