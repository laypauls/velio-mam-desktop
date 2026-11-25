// Categories: Happy / Boundary (0건, 대기 상한) / Concurrency (다른 스레드가 놓을 때 깨움) / Business rule (오류·패닉에도 감소)
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;

#[test]
fn t_aiexit_01_guard_counts_while_held_and_releases_on_drop() {
    // Arrange
    let f = InFlight::new();
    // Act
    let g1 = f.begin();
    let g2 = f.begin();
    let held = f.count();
    drop(g1);
    let after_one = f.count();
    drop(g2);
    // Assert
    assert_eq!((held, after_one, f.count()), (2, 1, 0));
}

#[test]
fn t_aiexit_02_guard_releases_on_error_return() {
    // Arrange
    let f = InFlight::new();
    let fail = |f: &InFlight| -> Result<(), &'static str> {
        let _g = f.begin();
        Err("reply was an error")
    };
    // Act
    let r = fail(&f);
    // Assert
    assert!(r.is_err());
    assert_eq!(f.count(), 0);
}

#[test]
fn t_aiexit_03_guard_releases_on_panic() {
    // Arrange
    let f = InFlight::new();
    // Act
    let r = catch_unwind(AssertUnwindSafe(|| {
        let _g = f.begin();
        panic!("executor bug");
    }));
    // Assert: 패닉 뒤에도 0 이고, 종료 대기가 영원히 걸리지 않는다
    assert!(r.is_err());
    assert_eq!(f.count(), 0);
    assert!(f.wait_idle(Duration::ZERO, |_| {}));
}

#[test]
fn t_aiexit_04_wait_idle_returns_immediately_when_nothing_is_in_flight() {
    // Arrange
    let f = InFlight::new();
    let mut called = false;
    // Act
    let idle = f.wait_idle(Duration::from_secs(60), |_| called = true);
    // Assert
    assert!(idle);
    assert!(!called, "기다릴 것이 없으면 안내도 없다");
}

#[test]
fn t_aiexit_05_wait_idle_wakes_when_the_request_finishes() {
    // Arrange
    let f = InFlight::new();
    let g = f.begin();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        drop(g);
    });
    let started = Instant::now();
    let mut seen = 0;
    // Act
    let idle = f.wait_idle(Duration::from_secs(30), |n| seen = n);
    // Assert
    t.join().unwrap();
    assert!(idle);
    assert_eq!(seen, 1);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn t_aiexit_06_wait_idle_gives_up_at_the_cap() {
    // Arrange: 놓이지 않는 요청
    let f = InFlight::new();
    let _g = f.begin();
    let started = Instant::now();
    // Act
    let idle = f.wait_idle(Duration::from_millis(80), |_| {});
    // Assert
    assert!(!idle);
    assert!(started.elapsed() >= Duration::from_millis(80));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(f.count(), 1);
}
