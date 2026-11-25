//! P2-9 비밀번호·인증 면제 시험 (T-prot-01~05).
//! 분류: Happy / Validation / Time / Business rule

mod common;
#[path = "common/protect_env.rs"]
mod protect_env;

use std::sync::Arc;

use common::env::AtomicClock;
use mam_core::protect::{
    LOCKOUT_MS, MAX_FAILURES, ProtectError, ProtectionGuard, SINGLE_USE_WINDOW_MS,
};
use protect_env::{PASSWORD, shared_hash};

fn guard() -> (Arc<AtomicClock>, ProtectionGuard) {
    let clock = Arc::new(AtomicClock::new());
    (clock.clone(), ProtectionGuard::new(clock))
}

fn code<T: std::fmt::Debug>(r: Result<T, ProtectError>) -> &'static str {
    r.unwrap_err().code()
}

#[test]
fn t_prot_01_set_then_verify_opens_the_exemption_window() {
    // Arrange
    let (_clock, g) = guard();
    let stored = g.set_password(None, PASSWORD).unwrap();

    // Act
    let before = g.require_auth(true);
    let status = g.verify_password(Some(&stored), PASSWORD, 5).unwrap();
    let after = g.require_auth(true);

    // Assert
    assert_eq!(before.unwrap_err().code(), "auth_required");
    assert!(status.password_set);
    assert_eq!(status.auth_valid_until, Some(common::T0 + 5 * 60_000));
    assert!(after.is_ok());
    assert_eq!(g.status(true).auth_valid_until, status.auth_valid_until);
}

#[test]
fn t_prot_02_validation_error_codes() {
    let (_c, g) = guard();
    let stored = shared_hash();

    assert_eq!(code(g.set_password(None, "1234567")), "invalid_input");
    assert_eq!(
        code(g.set_password(Some(stored), "another-password")),
        "password_already_set"
    );
    assert_eq!(
        code(g.change_password(None, PASSWORD, "new-password-1")),
        "password_not_set"
    );
    assert_eq!(
        code(g.verify_password(None, PASSWORD, 5)),
        "password_not_set"
    );
    assert_eq!(code(g.require_auth(false)), "password_not_set");
    assert_eq!(
        code(g.change_password(Some(stored), PASSWORD, "short")),
        "invalid_input"
    );
    assert_eq!(
        code(g.change_password(Some(stored), "wrong-current-pw", "new-password-1")),
        "password_incorrect"
    );
    // 정상 변경: 새 해시가 새 비밀번호로 검증된다
    let changed = g
        .change_password(Some(stored), PASSWORD, "new-password-1")
        .unwrap();
    assert!(
        g.verify_password(Some(&changed), "new-password-1", 5)
            .is_ok()
    );
    assert_eq!(
        code(g.verify_password(Some(&changed), PASSWORD, 5)),
        "password_incorrect"
    );
}

#[test]
fn t_prot_03_exemption_expires_with_injected_clock() {
    let (clock, g) = guard();
    g.verify_password(Some(shared_hash()), PASSWORD, 5).unwrap();

    clock.advance(5 * 60_000 - 1);
    assert!(g.require_auth(true).is_ok());
    clock.advance(1); // 정확히 만료 시각
    assert_eq!(code(g.require_auth(true)), "auth_required");
    assert_eq!(g.status(true).auth_valid_until, None);

    // 면제 0분 = 매번 인증: 방금 한 인증으로 한 번만 허용 (회귀: 예전에는 verify 직후에도 항상 auth_required)
    let s = g.verify_password(Some(shared_hash()), PASSWORD, 0).unwrap();
    assert_eq!(s.auth_valid_until, None, "UI must still prompt next time");
    assert!(g.require_auth(true).is_ok());
    assert_eq!(code(g.require_auth(true)), "auth_required");
    g.verify_password(Some(shared_hash()), PASSWORD, 0).unwrap();
    clock.advance(SINGLE_USE_WINDOW_MS);
    assert_eq!(
        code(g.require_auth(true)),
        "auth_required",
        "unused one-shot expires"
    );

    // revoke 는 즉시 끝낸다
    g.verify_password(Some(shared_hash()), PASSWORD, 5).unwrap();
    g.revoke();
    assert_eq!(code(g.require_auth(true)), "auth_required");
}

#[test]
fn t_prot_04_five_failures_lock_for_30s_and_hash_has_no_plaintext() {
    // Arrange
    let (clock, g) = guard();
    let stored = shared_hash();
    assert!(!stored.contains(PASSWORD));
    assert!(stored.starts_with("$argon2id$"));

    // Act: 연속 실패
    for _ in 0..MAX_FAILURES {
        assert_eq!(
            code(g.verify_password(Some(stored), "wrong-password", 5)),
            "password_incorrect"
        );
    }

    // Assert: 맞는 비밀번호도 잠금 중에는 거부된다
    match g.verify_password(Some(stored), PASSWORD, 5).unwrap_err() {
        ProtectError::TooManyAttempts { retry_after_ms } => assert_eq!(retry_after_ms, LOCKOUT_MS),
        other => panic!("expected TooManyAttempts, got {other:?}"),
    }
    assert_eq!(g.require_auth(true).unwrap_err().code(), "auth_required");
    clock.advance(LOCKOUT_MS - 1);
    assert_eq!(
        code(g.verify_password(Some(stored), PASSWORD, 5)),
        "too_many_attempts"
    );
    clock.advance(1);
    assert!(g.verify_password(Some(stored), PASSWORD, 5).is_ok());

    // 성공하면 카운터가 초기화된다: 다시 4번 틀려도 잠기지 않는다
    for _ in 0..MAX_FAILURES - 1 {
        let _ = g.verify_password(Some(stored), "wrong-password", 5);
    }
    assert!(g.verify_password(Some(stored), PASSWORD, 5).is_ok());
}

#[test]
fn t_prot_04b_failing_again_right_after_lockout_relocks_immediately() {
    let (clock, g) = guard();
    let stored = shared_hash();
    for _ in 0..MAX_FAILURES {
        let _ = g.verify_password(Some(stored), "wrong-password", 5);
    }
    clock.advance(LOCKOUT_MS);

    assert_eq!(
        code(g.verify_password(Some(stored), "wrong-password", 5)),
        "password_incorrect"
    );
    assert_eq!(
        code(g.verify_password(Some(stored), PASSWORD, 5)),
        "too_many_attempts"
    );
}

#[test]
fn t_prot_05_no_password_in_debug_display_or_error_messages() {
    // Arrange
    let (_c, g) = guard();
    let secret = "hunter2-very-secret";
    let stored = shared_hash();

    // Act: 가능한 모든 오류 경로와 출력 형식
    let mut texts = vec![format!("{g:?}")];
    for r in [
        g.verify_password(Some(stored), secret, 5).map(|_| ()),
        g.verify_password(None, secret, 5).map(|_| ()),
        g.change_password(Some(stored), secret, "short").map(|_| ()),
        g.change_password(Some(stored), secret, secret).map(|_| ()),
        g.set_password(Some(stored), secret).map(|_| ()),
        g.set_password(None, "short").map(|_| ()),
        g.verify_password(Some("garbage-hash"), secret, 5)
            .map(|_| ()),
    ] {
        let e = r.unwrap_err();
        texts.push(format!("{e:?}"));
        texts.push(e.to_string());
    }
    texts.push(format!("{:?}", g.status(true)));
    // 해시를 스스로 만든 경우의 PHC 에도 평문이 없다
    texts.push(g.set_password(None, secret).unwrap());

    // Assert
    for t in &texts {
        assert!(!t.contains(secret), "leaked in: {t}");
    }
    assert!(!texts.iter().any(|t| t.contains("garbage-hash")));
}
