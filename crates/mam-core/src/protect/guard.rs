//! 비밀번호 인증 상태 (D2) — 인증 면제 시간과 연속 실패 제한. 메모리에만 있고 앱 재시작 시 초기화된다.
//!
//! PHC 해시의 저장·로드는 호출자(settings)가 한다 — 이 타입은 해시를 받아 검증만 한다.
//! 시간은 주입된 시계로만 읽는다.

use std::sync::{Mutex, MutexGuard, PoisonError};

use super::password::{hash_password, validate_new_password, verify_password_hash};
use super::{ProtectError, ProtectionStatus};
use crate::SharedClock;

/// 이 횟수만큼 연속 실패하면 잠근다
pub const MAX_FAILURES: u32 = 5;
/// 잠금 시간 (ms)
pub const LOCKOUT_MS: i64 = 30_000;
/// 면제 0분일 때 방금 한 인증으로 보호 명령 한 번을 실행할 수 있는 시간 (ms) —
/// 비밀번호 확인과 명령 호출은 별개의 IPC 호출이라 이 틈이 없으면 어떤 보호 명령도 실행할 수 없다
pub const SINGLE_USE_WINDOW_MS: i64 = 60_000;

#[derive(Debug, Default)]
struct State {
    /// 인증 면제 만료 시각 (이 시각 전까지 유효)
    valid_until: Option<i64>,
    /// 면제 0분 설정으로 받은 인증 — 다음 보호 명령 한 번만 허용하고 소모된다
    single_use: bool,
    failures: u32,
    locked_until: i64,
}

pub struct ProtectionGuard {
    clock: SharedClock,
    state: Mutex<State>,
}

impl std::fmt::Debug for ProtectionGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProtectionGuard").finish_non_exhaustive()
    }
}

impl ProtectionGuard {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 최초 설정. 이미 설정돼 있으면 `PasswordAlreadySet`. 돌려주는 PHC 문자열을 호출자가 저장한다.
    pub fn set_password(&self, stored: Option<&str>, new: &str) -> Result<String, ProtectError> {
        if stored.is_some() {
            return Err(ProtectError::PasswordAlreadySet);
        }
        hash_password(new)
    }

    /// 변경. 현재 비밀번호 검증(실패 횟수에 포함)을 통과해야 한다. 인증 면제 상태는 바꾸지 않는다.
    pub fn change_password(
        &self,
        stored: Option<&str>,
        current: &str,
        new: &str,
    ) -> Result<String, ProtectError> {
        let stored = stored.ok_or(ProtectError::PasswordNotSet)?;
        validate_new_password(new)?;
        self.check(stored, current)?;
        hash_password(new)
    }

    /// 검증. 성공하면 `now + exempt_minutes` 까지 보호 명령을 허용한다.
    /// 0 = 면제 없음(매번 인증): 이 인증으로 보호 명령 **한 번**만 허용하고 상태에는 면제로 보이지 않는다.
    pub fn verify_password(
        &self,
        stored: Option<&str>,
        password: &str,
        exempt_minutes: u32,
    ) -> Result<ProtectionStatus, ProtectError> {
        let stored = stored.ok_or(ProtectError::PasswordNotSet)?;
        let mut st = self.lock();
        self.check_locked(&mut st, stored, password)?;
        let now = self.clock.now_ms();
        st.single_use = exempt_minutes == 0;
        let window = if st.single_use {
            SINGLE_USE_WINDOW_MS
        } else {
            i64::from(exempt_minutes) * 60_000
        };
        st.valid_until = Some(now.saturating_add(window));
        Ok(Self::status_of(&st, true, now))
    }

    /// 면제를 부여하지 않는 검증 (비밀번호 변경·초기화 확인용)
    pub fn check(&self, stored: &str, password: &str) -> Result<(), ProtectError> {
        let mut st = self.lock();
        self.check_locked(&mut st, stored, password)
    }

    fn check_locked(
        &self,
        st: &mut State,
        stored: &str,
        password: &str,
    ) -> Result<(), ProtectError> {
        let now = self.clock.now_ms();
        if now < st.locked_until {
            return Err(ProtectError::TooManyAttempts {
                retry_after_ms: st.locked_until - now,
            });
        }
        match verify_password_hash(stored, password) {
            Ok(()) => {
                st.failures = 0;
                st.locked_until = 0;
                Ok(())
            }
            Err(ProtectError::PasswordIncorrect) => {
                st.failures = st.failures.saturating_add(1);
                if st.failures >= MAX_FAILURES {
                    st.locked_until = now.saturating_add(LOCKOUT_MS);
                }
                Err(ProtectError::PasswordIncorrect)
            }
            Err(other) => Err(other),
        }
    }

    /// 보호 명령 진입 검사. 비밀번호 미설정이면 `PasswordNotSet`, 면제 시간 밖이면 `AuthRequired`.
    /// 면제 0분(1회용) 인증은 여기서 소모된다 — 입력 검증을 먼저 끝낸 뒤에 불러야 한다
    /// (검증 전 사전 확인은 소모하지 않는 [`Self::check_auth`]).
    pub fn require_auth(&self, password_set: bool) -> Result<(), ProtectError> {
        self.auth(password_set, true)
    }

    /// [`Self::require_auth`] 와 같은 검사지만 1회용 인증을 소모하지 않는다.
    pub fn check_auth(&self, password_set: bool) -> Result<(), ProtectError> {
        self.auth(password_set, false)
    }

    fn auth(&self, password_set: bool, consume: bool) -> Result<(), ProtectError> {
        if !password_set {
            return Err(ProtectError::PasswordNotSet);
        }
        let now = self.clock.now_ms();
        let mut st = self.lock();
        match st.valid_until {
            Some(until) if now < until => {
                if consume && st.single_use {
                    st.valid_until = None;
                    st.single_use = false;
                }
                Ok(())
            }
            _ => Err(ProtectError::AuthRequired),
        }
    }

    /// 인증 면제를 즉시 끝낸다
    pub fn revoke(&self) {
        let mut st = self.lock();
        st.valid_until = None;
        st.single_use = false;
    }

    pub fn status(&self, password_set: bool) -> ProtectionStatus {
        let now = self.clock.now_ms();
        Self::status_of(&self.lock(), password_set, now)
    }

    fn status_of(st: &State, password_set: bool, now: i64) -> ProtectionStatus {
        ProtectionStatus {
            password_set,
            auth_valid_until: st.valid_until.filter(|&u| now < u && !st.single_use),
        }
    }
}
