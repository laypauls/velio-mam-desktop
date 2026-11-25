//! 일시적 실패 분류 시험 — 취소·`--version` 제한 시간은 영구 코드(`cli_not_found`/`cli_unsupported_version`)가 아니다.
//! 주기 탐지가 확인된 경로를 매번 지우지 않는다.
//! 분류: External / Time / Concurrency(취소) / Business rule

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::external::cli::CliClient;
use crate::external::cli::fake::FakeRunner;
use crate::external::client::AiClient;
use crate::external::consent::ConsentGate;
use crate::external::error::ExternalError;

fn mk(scripts: Scripts) -> (tempfile::TempDir, Arc<FakeRunner>, Arc<CliHost>) {
    let dir = tempfile::tempdir().unwrap();
    let runner = dispatch(scripts);
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    h.set_probe_timeout(Duration::from_millis(150));
    (dir, runner, h)
}

#[test]
fn t_aicli_tr_01_version_probe_timeout_is_retryable_not_unsupported() {
    // Arrange: 응답 없는 --version
    let (_d, _r, h) = mk(Scripts {
        version: Some(vec![Step::Hang]),
        ..Default::default()
    });

    // Act
    let e = h.ensure_ready().unwrap_err();

    // Assert
    assert_eq!(e, ExternalError::Timeout);
    assert_eq!(e.error_code(), "timeout");
}

#[test]
fn t_aicli_tr_02_analyze_during_a_version_timeout_reports_timeout_and_spawns_no_request() {
    let (_d, runner, h) = mk(Scripts {
        version: Some(vec![Step::Hang]),
        ..Default::default()
    });
    let c = CliClient::new(h, None);

    let e = c
        .analyze(&ConsentGate::check(true).unwrap(), &request())
        .unwrap_err();

    assert_eq!(e.error_code(), "timeout");
    assert_eq!(runner.count_with_arg("-p"), 0);
}

#[test]
fn t_aicli_tr_03_cancel_during_the_version_probe_is_cancelled() {
    // Arrange
    let (_d, runner, h) = mk(Scripts {
        version: Some(vec![Step::Hang]),
        ..Default::default()
    });
    h.set_probe_timeout(Duration::from_secs(30));
    let t = {
        let h = Arc::clone(&h);
        std::thread::spawn(move || h.ensure_ready())
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while runner.spawn_count() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }

    // Act
    h.kill_all();
    let e = t.join().unwrap().unwrap_err();

    // Assert: 종료 경로가 만든 중단은 CLI 문제가 아니다. 이후 호출도 마찬가지다.
    assert_eq!(e, ExternalError::Cancelled);
    assert_eq!(h.ensure_ready().unwrap_err(), ExternalError::Cancelled);
}

#[test]
fn t_aicli_tr_04_unparseable_or_old_version_is_still_unsupported() {
    // 회귀 대조: 판정이 난 경우는 그대로 영구 코드
    for out in ["not a version\n", "2.1.100\n"] {
        let (_d, _r, h) = mk(Scripts {
            version: Some(vec![Step::Out(out.into()), Step::Exit(Some(0))]),
            ..Default::default()
        });
        assert_eq!(
            h.ensure_ready().unwrap_err(),
            ExternalError::CliUnsupportedVersion,
            "{out}"
        );
    }
    let (_d, _r, h) = mk(Scripts {
        version: Some(vec![Step::Exit(Some(1))]),
        ..Default::default()
    });
    assert_eq!(
        h.ensure_ready().unwrap_err(),
        ExternalError::CliUnsupportedVersion
    );
}

#[test]
fn t_aicli_tr_05_transient_detect_failure_keeps_the_verified_path() {
    // Arrange: 처음엔 정상, 이후 --version 이 멈춘다
    let hang = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&hang);
    let base = Scripts::default();
    let runner = FakeRunner::new(move |c| {
        let a = args_of(c);
        match a.first().map(String::as_str) {
            Some("--version") if flag.load(Ordering::SeqCst) => Some(vec![Step::Hang]),
            Some("--version") => base.version.clone(),
            Some("auth") => Some(base.status.clone()),
            _ => None,
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    h.set_probe_timeout(Duration::from_millis(150));
    h.ensure_ready().unwrap();
    let probes = runner.count_with_arg("--version");

    // Act: 주기 탐지가 제한 시간에 걸린다
    hang.store(true, Ordering::SeqCst);
    let st = h.detect();

    // Assert: 탐지 한 번(+1 호출)만 일어났고, 확인된 경로는 유지돼 요청 직전 확인이 --version 을 다시 돌리지 않는다
    assert!(st.found);
    assert_eq!(runner.count_with_arg("--version"), probes + 1);
    assert!(h.ensure_ready().is_ok());
    assert_eq!(runner.count_with_arg("--version"), probes + 1);
}

#[test]
fn t_aicli_tr_06_definitive_unsupported_detect_clears_the_verified_path() {
    // Arrange: 확인된 뒤 CLI 가 낮은 버전으로 바뀌었다
    let old = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&old);
    let base = Scripts::default();
    let runner = FakeRunner::new(move |c| {
        let a = args_of(c);
        match a.first().map(String::as_str) {
            Some("--version") if flag.load(Ordering::SeqCst) => {
                Some(vec![Step::Out("2.1.100\n".into()), Step::Exit(Some(0))])
            }
            Some("--version") => base.version.clone(),
            Some("auth") => Some(base.status.clone()),
            _ => None,
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let h = host(Arc::clone(&runner), FakeEnv::with_claude(), dir.path());
    h.ensure_ready().unwrap();

    // Act
    old.store(true, Ordering::SeqCst);
    let st = h.detect();

    // Assert
    assert!(st.found && !st.supported);
    assert_eq!(
        h.ensure_ready().unwrap_err(),
        ExternalError::CliUnsupportedVersion
    );
}
