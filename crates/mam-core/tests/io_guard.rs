//! `io_guard` — 막힐 수 있는 파일 호출의 제한 시간·합류·볼륨별 상한 (T-ioguard-01 ~ 08, 15).
//!
//! 분류: Happy (01) / External (02 막힌 호출, 07 호출 패닉) / Concurrency (03 같은 키 합류, 04 막힌 키 재검사 없음) /
//! Boundary (05 볼륨별 상한, 06 스레드 쌓임 없음) / 진단 (08 누계) / Concurrency (15 자리 반납이 결과 통지보다 먼저).
//! 실제 네트워크 공유에 의존하지 않는다 — 막히는 호출은 문 열림(`Gate`)을 기다리는 가짜이고, 제한 시간은 짧게 준다.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use mam_core::scan::{Guarded, IoGuard, IoGuardConfig};

const SHORT: Duration = Duration::from_millis(80);
const LONG: Duration = Duration::from_secs(10);
/// 시험이 실패해도 막힌 스레드가 영구히 남지 않게 하는 상한
const SAFETY: Duration = Duration::from_secs(30);

/// 열기 전까지 호출을 막는 가짜 — "응답 없는 공유"
#[derive(Default)]
struct Gate(Mutex<bool>, Condvar);

impl Gate {
    fn wait(&self) {
        let g = self.0.lock().unwrap();
        let _ = self.1.wait_timeout_while(g, SAFETY, |open| !*open).unwrap();
    }
    fn open(&self) {
        *self.0.lock().unwrap() = true;
        self.1.notify_all();
    }
}

fn guard(max_per_volume: usize) -> Arc<IoGuard> {
    IoGuard::new(IoGuardConfig {
        timeout: SHORT,
        max_per_volume,
    })
}

/// 조건이 될 때까지 짧게 기다린다 (스레드 정리 대기 전용, 상한 있음)
fn eventually(mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}

#[test]
fn t_ioguard_01_a_call_that_finishes_in_time_returns_its_value_and_leaves_no_thread() {
    // Arrange
    let io = guard(2);

    // Act
    let r = io.guarded("k", 1, LONG, || 7);

    // Assert
    assert_eq!(r, Guarded::Done(7));
    assert!(eventually(|| io.live_checks(1) == 0));
    assert!(!io.is_unresponsive(1));
}

#[test]
fn t_ioguard_02_a_blocked_call_times_out_and_its_thread_is_released_when_it_finally_returns() {
    // Arrange
    let io = guard(2);
    let gate = Arc::new(Gate::default());
    let g = Arc::clone(&gate);

    // Act
    let started = Instant::now();
    let r = io.guarded("k", 1, SHORT, move || {
        g.wait();
        1
    });

    // Assert: 제한 시간 안에 돌아왔고(스레드는 아직 막혀 있다), 풀리면 정리된다
    assert_eq!(r, Guarded::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(io.live_checks(1), 1);
    gate.open();
    assert!(eventually(|| io.live_checks(1) == 0), "thread count recovers");
}

#[test]
fn t_ioguard_03_concurrent_checks_of_the_same_key_run_the_call_once_and_share_the_result() {
    // Arrange
    let io = guard(4);
    let gate = Arc::new(Gate::default());
    let runs = Arc::new(AtomicUsize::new(0));
    let call = |io: Arc<IoGuard>, gate: Arc<Gate>, runs: Arc<AtomicUsize>| {
        std::thread::spawn(move || {
            io.guarded("same", 1, LONG, move || {
                runs.fetch_add(1, Ordering::SeqCst);
                gate.wait();
                42
            })
        })
    };

    // Act: 첫 호출이 막혀 있는 동안 같은 키로 한 번 더 건다
    let first = call(Arc::clone(&io), Arc::clone(&gate), Arc::clone(&runs));
    assert!(eventually(|| io.live_checks(1) == 1));
    let second = call(Arc::clone(&io), Arc::clone(&gate), Arc::clone(&runs));
    std::thread::sleep(Duration::from_millis(100));
    gate.open();

    // Assert: 호출은 한 번만 실행되고 둘 다 같은 결과를 받는다
    assert_eq!(first.join().unwrap(), Guarded::Done(42));
    assert_eq!(second.join().unwrap(), Guarded::Done(42));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[test]
fn t_ioguard_04_rechecking_a_hung_key_does_not_start_another_call() {
    // Arrange: 첫 검사가 제한 시간을 넘겨 아직 막혀 있다
    let io = guard(4);
    let gate = Arc::new(Gate::default());
    let runs = Arc::new(AtomicUsize::new(0));
    let attempt = |io: &Arc<IoGuard>| {
        let (g, r) = (Arc::clone(&gate), Arc::clone(&runs));
        io.guarded("hung", 1, SHORT, move || {
            r.fetch_add(1, Ordering::SeqCst);
            g.wait();
        })
    };
    assert_eq!(attempt(&io), Guarded::TimedOut);

    // Act: 같은 키로 여러 번 다시 건다
    let again: Vec<_> = (0..5).map(|_| attempt(&io)).collect();

    // Assert: 스레드도 호출도 늘지 않는다
    assert!(again.iter().all(|r| *r == Guarded::TimedOut));
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert_eq!(io.live_checks(1), 1);
    gate.open();
    assert!(eventually(|| io.live_checks(1) == 0));
}

#[test]
fn t_ioguard_05_a_volume_at_its_cap_is_unresponsive_and_new_checks_return_without_a_thread() {
    // Arrange: 상한 2, 서로 다른 키 2개가 막혀 있다
    let io = guard(2);
    let gate = Arc::new(Gate::default());
    let runs = Arc::new(AtomicUsize::new(0));
    for key in ["a", "b"] {
        let (g, r) = (Arc::clone(&gate), Arc::clone(&runs));
        assert_eq!(
            io.guarded(key, 7, SHORT, move || {
                r.fetch_add(1, Ordering::SeqCst);
                g.wait();
            }),
            Guarded::TimedOut
        );
    }
    assert!(io.is_unresponsive(7));
    assert_eq!(io.unresponsive_volumes(), vec![7]);

    // Act: 같은 볼륨에 새 검사 — 곧바로 돌아오고 호출은 시작되지 않는다
    let r = runs.clone();
    let started = Instant::now();
    let third = io.guarded("c", 7, LONG, move || {
        r.fetch_add(1, Ordering::SeqCst);
    });

    // Assert
    assert_eq!(third, Guarded::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(2), "returns immediately, not after the timeout");
    assert!(eventually(|| runs.load(Ordering::SeqCst) == 2));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(runs.load(Ordering::SeqCst), 2, "the third call never started");
    // 다른 볼륨은 영향이 없다
    assert_eq!(io.guarded("x", 8, LONG, || 5), Guarded::Done(5));
    assert!(!io.is_unresponsive(8));
    // 막힌 호출이 풀리면 회복한다
    gate.open();
    assert!(eventually(|| !io.is_unresponsive(7)), "recovers once blocked calls return");
    assert_eq!(io.guarded("d", 7, LONG, || 9), Guarded::Done(9));
}

#[test]
fn t_ioguard_06_hammering_a_dead_volume_never_piles_up_threads_past_the_cap() {
    // Arrange
    let cap = 3;
    let io = guard(cap);
    let gate = Arc::new(Gate::default());
    let started = Arc::new(AtomicUsize::new(0));

    // Act: 서로 다른 키 50개를 막힌 볼륨에 건다
    let mut peak = 0;
    for n in 0..50 {
        let (g, s) = (Arc::clone(&gate), Arc::clone(&started));
        let r = io.guarded(&format!("k{n}"), 1, Duration::from_millis(5), move || {
            s.fetch_add(1, Ordering::SeqCst);
            g.wait();
        });
        assert_eq!(r, Guarded::TimedOut);
        peak = peak.max(io.live_checks(1));
    }

    // Assert
    assert!(peak <= cap, "peak {peak} > cap {cap}");
    assert!(eventually(|| started.load(Ordering::SeqCst) == cap), "exactly `cap` calls were started");
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(started.load(Ordering::SeqCst), cap, "no call beyond the cap ever starts");
    gate.open();
    assert!(eventually(|| io.live_checks(1) == 0));
}

#[test]
fn t_ioguard_07_a_panicking_call_is_a_timeout_and_frees_its_slot() {
    // Arrange
    let io = guard(1);

    // Act
    let r: Guarded<i32> = io.guarded("boom", 1, LONG, || panic!("simulated fs panic"));

    // Assert: 호출 쪽으로 패닉이 번지지 않고, 볼륨이 응답 없음으로 굳지 않는다
    assert_eq!(r, Guarded::TimedOut);
    assert!(eventually(|| io.live_checks(1) == 0));
    assert!(!io.is_unresponsive(1));
    assert_eq!(io.guarded("ok", 1, LONG, || 3), Guarded::Done(3));
}

#[test]
fn t_ioguard_08_timeouts_are_counted_for_diagnostics() {
    // Arrange
    let io = guard(1);
    let gate = Arc::new(Gate::default());
    let g = Arc::clone(&gate);
    assert_eq!(io.timeout_count(), 0);

    // Act: 제한 시간 초과 1회 + 상한 때문에 걸지 않은 1회
    let _ = io.guarded("a", 1, SHORT, move || g.wait());
    let _ = io.guarded("b", 1, SHORT, || ());

    // Assert
    assert_eq!(io.timeout_count(), 2);
    gate.open();
    assert!(eventually(|| io.live_checks(1) == 0));
}

#[test]
fn t_ioguard_15_back_to_back_healthy_calls_on_a_cap_one_volume_never_time_out() {
    // Arrange: 상한 1 — 직전 검사 스레드가 자리를 반납하기 전에 다음 검사가 들어오면 "응답 없음" 이 되던 경합
    let io = guard(1);

    // Act / Assert: 결과를 받자마자 곧바로 다음 검사를 건다 (키가 다른 경우와 같은 경우)
    for n in 0..500 {
        assert_eq!(io.guarded(&format!("k{n}"), 1, LONG, move || n), Guarded::Done(n), "distinct key, iteration {n}");
    }
    for n in 0..500 {
        assert_eq!(io.guarded("same", 1, LONG, move || n), Guarded::Done(n), "same key, iteration {n}");
    }
    // 패닉한 호출의 뒤에도 자리는 이미 비어 있다 (결과 통지 전에 반납)
    for n in 0..30 {
        let r: Guarded<i32> = io.guarded("boom", 1, LONG, || panic!("simulated fs panic"));
        assert_eq!(r, Guarded::TimedOut);
        assert_eq!(io.guarded("after", 1, LONG, || 1), Guarded::Done(1), "after panic, iteration {n}");
    }
    // 정상 호출은 막힘으로 세지 않는다 — 시간 초과 누계는 패닉 30건뿐
    assert_eq!(io.timeout_count(), 30);
}
