//! T-aiexit-10~14 - 종료 중 진행 중인 외부 AI 요청: 응답·사용량 저장까지 기다리고, 정지 뒤에는 새 요청을 보내지 않는다.
//! 분류: Business rule / Concurrency / External(오류·패닉) / Boundary(요청 없음).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ai_env::{AiEnv, ok_external};
use mam_core::ai_jobs::{
    AiDeps, AiExecutor, AiJobsConfig, ExternalAnalyzer, ExternalRequest, ExternalResult, InFlight,
    StepFailure,
};
use mam_core::jobs::{JobKind, JobOutcome};

enum Reply {
    Ok,
    Err,
    Panic,
}

/// 응답을 `release` 로 풀어 줄 때까지 막히는 외부 AI 스텁 (실제 HTTP 없음).
struct GatedExternal {
    sent: Sender<()>,
    release: Mutex<Receiver<Reply>>,
    calls: AtomicUsize,
}
impl ExternalAnalyzer for GatedExternal {
    fn provider(&self) -> String {
        "openai".into()
    }
    fn analyze(&self, _req: &ExternalRequest) -> Result<ExternalResult, StepFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.sent.send(());
        match self.release.lock().unwrap().recv().unwrap() {
            Reply::Ok => Ok(ok_external()),
            Reply::Err => Err(StepFailure::new("server_error", "5xx")),
            Reply::Panic => panic!("analyzer bug"),
        }
    }
}

struct Gated {
    e: Arc<AiEnv>,
    ext: Arc<GatedExternal>,
    exec: Arc<AiExecutor>,
    in_flight: InFlight,
    sent: Receiver<()>,
    release: Sender<Reply>,
}

fn gated() -> Gated {
    let e = Arc::new(AiEnv::new());
    let (sent_tx, sent) = channel();
    let (release, release_rx) = channel();
    let ext = Arc::new(GatedExternal {
        sent: sent_tx,
        release: Mutex::new(release_rx),
        calls: AtomicUsize::new(0),
    });
    let in_flight = InFlight::new();
    let exec = Arc::new(
        AiExecutor::new(
            AiDeps {
                pool: Arc::clone(&e.env.pool),
                clock: e.env.clock.clone(),
                settings: e.settings.clone(),
                external: ext.clone(),
                stt: e.stt.clone(),
                embedding: e.emb.clone(),
                utc_offset: Arc::new(|| 0),
            },
            AiJobsConfig::new(e.cache.clone()),
        )
        .with_in_flight(in_flight.clone()),
    );
    Gated {
        e,
        ext,
        exec,
        in_flight,
        sent,
        release,
    }
}

impl Gated {
    /// 작업 스레드: 1건 꺼내 실행하고 결과를 큐에 반영한다 (러너와 같은 순서).
    fn spawn_job(
        &self,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<Result<JobOutcome, mam_core::jobs::JobError>> {
        let (e, exec) = (Arc::clone(&self.e), Arc::clone(&self.exec));
        std::thread::spawn(move || {
            let job = e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
            let res = exec.run(&job, &|| stop.load(Ordering::SeqCst));
            e.queue.finish(&job, res.clone()).unwrap();
            res
        })
    }
}

#[test]
fn t_aiexit_10_exit_waits_for_the_inflight_reply_and_records_usage_without_requeue() {
    // Arrange: 요청이 나간 상태에서 응답이 막혀 있다
    let g = gated();
    let id = g.e.add_asset("image");
    g.e.queue_all(id, None, false);
    let stop = Arc::new(AtomicBool::new(false));
    let job = g.spawn_job(Arc::clone(&stop));
    g.sent.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(g.in_flight.count(), 1);

    // Act: 종료 = 정지 신호 -> 대기. 응답은 대기가 시작된 뒤에 풀린다
    stop.store(true, Ordering::SeqCst);
    let release = g.release.clone();
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        release.send(Reply::Ok).unwrap();
    });
    let mut waited = false;
    let idle = g
        .in_flight
        .wait_idle(Duration::from_secs(30), |_| waited = true);
    let outcome = job.join().unwrap();
    late.join().unwrap();

    // Assert: 응답·사용량이 저장되고 재전송이 없으며, 시도 횟수를 쓰지 않는다
    assert!(waited && idle);
    assert_eq!(g.ext.calls.load(Ordering::SeqCst), 1, "재전송 없음");
    assert_eq!(
        g.e.env.count("SELECT COUNT(*) FROM ai_usage WHERE ok = 1"),
        1
    );
    assert_eq!(g.e.st(id, "describe"), "done");
    assert_eq!(outcome, Ok(JobOutcome::Yielded), "다음 단계는 정지로 양보");
    assert_eq!(
        g.e.env
            .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'"),
        0
    );
    assert_eq!(g.in_flight.count(), 0);
}

#[test]
fn t_aiexit_11_no_inflight_request_means_no_wait() {
    // Arrange
    let g = gated();
    let mut waited = false;
    // Act
    let started = std::time::Instant::now();
    let idle = g
        .in_flight
        .wait_idle(Duration::from_secs(60), |_| waited = true);
    // Assert
    assert!(idle && !waited);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn t_aiexit_12_job_that_has_not_sent_yet_yields_after_stop_without_sending() {
    // Arrange: 외부 단계 직전 확인(1번째)은 통과하고, 요청 직전 확인(2번째)에서 정지가 서 있다
    let g = gated();
    let id = g.e.add_asset("image");
    g.e.queue_all(id, None, false);
    let calls = AtomicUsize::new(0);
    let job = g.e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();

    // Act
    let res = g
        .exec
        .run(&job, &|| calls.fetch_add(1, Ordering::SeqCst) >= 1);
    g.e.queue.finish(&job, res.clone()).unwrap();

    // Assert: 보내지 않았고, 단계는 queued, 시도·사용량·실패 없음, 작업은 pending 으로 돌아간다
    assert_eq!(res, Ok(JobOutcome::Yielded));
    assert_eq!(g.ext.calls.load(Ordering::SeqCst), 0);
    assert_eq!(g.e.st(id, "describe"), "queued");
    assert_eq!(g.e.env.count("SELECT COUNT(*) FROM ai_usage"), 0);
    assert_eq!(
        g.e.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'"),
        1
    );
    assert_eq!(
        g.e.env
            .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'"),
        0
    );
    assert_eq!(g.in_flight.count(), 0, "양보한 작업은 계수를 남기지 않는다");
}

#[test]
fn t_aiexit_13_guard_is_released_after_an_error_reply_is_recorded() {
    // Arrange
    let g = gated();
    let id = g.e.add_asset("image");
    g.e.queue_all(id, None, false);
    let job = g.spawn_job(Arc::new(AtomicBool::new(false)));
    g.sent.recv_timeout(Duration::from_secs(10)).unwrap();

    // Act
    g.release.send(Reply::Err).unwrap();
    let res = job.join().unwrap();

    // Assert: 실패 사용량이 저장된 뒤에 계수가 0 이다 (오류는 재시도 대상으로 전달)
    assert!(res.is_err());
    assert_eq!(g.in_flight.count(), 0);
    assert_eq!(
        g.e.env.count("SELECT COUNT(*) FROM ai_usage WHERE ok = 0"),
        1
    );
}

#[test]
fn t_aiexit_14_guard_is_released_when_the_analyzer_panics() {
    // Arrange
    let g = gated();
    let id = g.e.add_asset("image");
    g.e.queue_all(id, None, false);
    let job = g.e.queue.claim_next(&[JobKind::Ai]).unwrap().unwrap();
    g.release.send(Reply::Panic).unwrap();

    // Act
    let r = catch_unwind(AssertUnwindSafe(|| g.exec.run(&job, &|| false)));

    // Assert
    assert!(r.is_err());
    assert_eq!(g.in_flight.count(), 0);
    assert!(g.in_flight.wait_idle(Duration::ZERO, |_| {}));
}
