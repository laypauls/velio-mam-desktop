//! 구독 한도 일시정지 시험 — 한도에 걸리면 해제 시각까지 CLI 를 띄우지 않고, 시계가 지나면 재개한다. 수동 연결 확인은 막지 않는다.
//! 분류: Business rule / Time / Boundary(해제 시각 불명·상한) / Concurrency(반복 요청)

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::external::cli::CliClient;
use crate::external::cli::fake::FakeRunner;
use crate::external::client::AiClient;
use crate::external::consent::ConsentGate;
use crate::external::error::ExternalError;
use crate::external::types::{UsageRecord, UsageSink};

const NOW_MS: i64 = 1_000_000_000_000;

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct Sink(Mutex<Vec<UsageRecord>>);
impl UsageSink for Sink {
    fn record(&self, u: &UsageRecord) {
        self.0.lock().unwrap().push(u.clone());
    }
}

fn gate() -> ConsentGate {
    ConsentGate::check(true).unwrap()
}

fn limited_script(resets_at: Option<i64>) -> Vec<Step> {
    let mut v = vec![line(INIT_SUB)];
    if let Some(r) = resets_at {
        v.push(line(&format!(
            r#"{{"type":"rate_limit_event","rate_limit_info":{{"status":"rejected","resetsAt":{r}}}}}"#
        )));
    }
    v.push(line(&error_result("429", "limit")));
    v.push(Step::Exit(Some(1)));
    v
}

struct Fx {
    _dir: tempfile::TempDir,
    runner: Arc<FakeRunner>,
    clock: Arc<TestClock>,
    host: Arc<CliHost>,
    sink: Arc<Sink>,
    /// 켜면 요청이 한도 거부로 끝난다 (꺼지면 성공)
    limited: Arc<AtomicBool>,
}

fn fx(resets_at: Option<i64>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let limited = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&limited);
    let base = Scripts::default();
    let runner = FakeRunner::new(move |c| {
        let a = args_of(c);
        match a.first().map(String::as_str) {
            Some("--version") => base.version.clone(),
            Some("auth") if a.get(1).map(String::as_str) == Some("status") => {
                Some(base.status.clone())
            }
            Some("-p") if flag.load(Ordering::SeqCst) => Some(limited_script(resets_at)),
            Some("-p") => Some(success_request()),
            _ => None,
        }
    });
    let clock = Arc::new(TestClock(AtomicI64::new(NOW_MS)));
    let host = CliHost::new(
        Arc::clone(&runner) as _,
        Arc::new(FakeEnv::with_claude()),
        Arc::clone(&clock) as _,
        dir.path().join("ai-cli"),
    );
    Fx {
        _dir: dir,
        runner,
        clock,
        host,
        sink: Arc::default(),
        limited,
    }
}

impl Fx {
    fn client(&self) -> CliClient {
        CliClient::new(Arc::clone(&self.host), Some(Arc::clone(&self.sink) as _))
    }
    fn spawns(&self) -> usize {
        self.runner.count_with_arg("-p")
    }
}

#[test]
fn t_aicli_lim_01_limit_pauses_until_the_reset_time_and_later_runs_do_not_spawn() {
    // Arrange: 2시간 뒤에 풀린다
    let f = fx(Some(NOW_MS / 1000 + 7200));
    let c = f.client();

    // Act 1: 첫 요청이 한도에 걸린다
    let e = c.analyze(&gate(), &request()).unwrap_err();

    // Assert 1
    assert_eq!(
        e,
        ExternalError::SubscriptionLimit {
            retry_after_secs: Some(7200)
        }
    );
    assert_eq!(f.host.paused_until_ms(), Some(NOW_MS + 7_200_000));
    assert_eq!(f.spawns(), 1);

    // Act 2: 이후 요청 - 프로세스 없음, 남은 시간을 알려 준다, 사용량 행도 없다
    f.clock.0.fetch_add(1_000_000, Ordering::SeqCst);
    for _ in 0..3 {
        let e = c.analyze(&gate(), &request()).unwrap_err();
        assert_eq!(
            e,
            ExternalError::SubscriptionLimit {
                retry_after_secs: Some(6200)
            }
        );
    }
    assert_eq!(f.spawns(), 1, "일시정지 중에는 CLI 를 띄우지 않는다");
    assert_eq!(
        f.sink.0.lock().unwrap().len(),
        1,
        "요청이 없었으니 사용량도 없다"
    );
}

#[test]
fn t_aicli_lim_02_after_the_reset_time_requests_run_again() {
    // Arrange
    let f = fx(Some(NOW_MS / 1000 + 3600));
    let c = f.client();
    c.analyze(&gate(), &request()).unwrap_err();
    f.limited.store(false, Ordering::SeqCst); // 한도가 풀렸다

    // Act 1: 해제 1초 전 - 아직 막혀 있다
    f.clock.0.fetch_add(3_599_000, Ordering::SeqCst);
    assert!(c.analyze(&gate(), &request()).is_err());
    assert_eq!(f.spawns(), 1);

    // Act 2: 해제 시각 이후
    f.clock.0.fetch_add(1_000, Ordering::SeqCst);
    let out = c.analyze(&gate(), &request());

    // Assert
    assert!(out.is_ok(), "{out:?}");
    assert_eq!(f.spawns(), 2);
    assert_eq!(f.host.paused_until_ms(), None);
}

#[test]
fn t_aicli_lim_03_unknown_reset_time_pauses_for_one_hour() {
    let f = fx(None);

    let e = f.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(
        e,
        ExternalError::SubscriptionLimit {
            retry_after_secs: None
        }
    );
    assert_eq!(f.host.paused_until_ms(), Some(NOW_MS + 3_600_000));
}

#[test]
fn t_aicli_lim_04_far_reset_time_is_capped_at_eight_days() {
    let f = fx(Some(NOW_MS / 1000 + 30 * 86_400));

    let e = f.client().analyze(&gate(), &request()).unwrap_err();

    assert_eq!(
        e,
        ExternalError::SubscriptionLimit {
            retry_after_secs: Some(8 * 86_400)
        }
    );
    assert_eq!(f.host.paused_until_ms(), Some(NOW_MS + 8 * 86_400_000));
}

#[test]
fn t_aicli_lim_05_manual_check_still_runs_while_paused_and_success_clears_the_pause() {
    // Arrange
    let f = fx(Some(NOW_MS / 1000 + 7200));
    let c = f.client();
    c.analyze(&gate(), &request()).unwrap_err();
    assert!(f.host.paused_until_ms().is_some());
    f.limited.store(false, Ordering::SeqCst);

    // Act: 사용자가 연결 확인을 누른다
    let r = c.check(&gate(), None);

    // Assert
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(f.spawns(), 2, "확인은 일시정지와 무관하게 실행된다");
    assert_eq!(f.host.paused_until_ms(), None);
}

#[test]
fn t_aicli_lim_06_pause_only_extends_never_shortens() {
    let f = fx(None);
    f.host.pause_for_secs(7200);
    f.host.pause_for_secs(60);

    assert_eq!(f.host.paused_until_ms(), Some(NOW_MS + 7_200_000));
}

#[test]
fn t_aicli_lim_07_non_limit_failures_do_not_pause() {
    // 회귀 대조: 서버 오류 같은 일반 실패는 일시정지를 만들지 않는다
    let dir = tempfile::tempdir().unwrap();
    let s = Scripts {
        request: vec![
            line(INIT_SUB),
            line(&error_result("503", "down")),
            Step::Exit(Some(1)),
        ],
        ..Default::default()
    };
    let h = host(dispatch(s), FakeEnv::with_claude(), dir.path());
    let c = CliClient::new(Arc::clone(&h), None);

    let e = c.analyze(&gate(), &request()).unwrap_err();

    assert!(matches!(e, ExternalError::Server { .. }), "{e:?}");
    assert_eq!(h.paused_until_ms(), None);
}
