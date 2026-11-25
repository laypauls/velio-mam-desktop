//! 막힐 수 있는 파일 호출의 제한 시간 (watch_folder `guard` 와 같은 생각, 결정 F).
//!
//! Rust 의 막힌 파일 호출은 취소할 수 없다 — 제한 시간을 걸어도 작업 스레드는 뒤에서 계속 막혀 있다.
//! 끊긴 공유에서 검사를 계속 걸면 스레드가 쌓이므로 두 가지로 막는다.
//! - **같은 키의 검사는 하나만** 건다: 진행 중인 검사가 있으면 새 스레드를 만들지 않고 그 결과를 기다린다(합류)
//! - **볼륨별 동시 검사 상한**: 막혀 있는 스레드를 포함해 상한에 닿은 볼륨은 "응답 없음" 으로 보고, 새 검사를
//!   스레드를 만들지 않고 곧바로 [`Guarded::TimedOut`] 으로 돌려준다 (막힌 호출이 풀려 수가 내려가면 회복)
//!
//! `TimedOut` 은 "지금 알 수 없음" 이지 "없음" 이 아니다 — 호출자는 이 파일을 missing 으로 만들지 않는다 (연결 끊김 ≠ 삭제).

use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// 기본 제한 시간 (watch_folder 와 같은 20초)
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);
/// 볼륨 하나에서 동시에 살아 있을 수 있는 검사 스레드 수 (막힌 것 포함)
pub const DEFAULT_MAX_PER_VOLUME: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoGuardConfig {
    pub timeout: Duration,
    pub max_per_volume: usize,
}

impl Default for IoGuardConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_per_volume: DEFAULT_MAX_PER_VOLUME,
        }
    }
}

/// 제한 시간 안에 끝났는가.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guarded<T> {
    Done(T),
    /// 제한 시간을 넘겼거나, 볼륨이 응답 없음 상태라 검사를 걸지 않았다 (호출 쪽 패닉도 같이 취급)
    TimedOut,
}

enum SlotState<T> {
    Running,
    Done(T),
    Failed,
}

struct Slot<T> {
    state: Mutex<SlotState<T>>,
    cv: Condvar,
}

#[derive(Default)]
struct Registry {
    /// 진행 중인 검사 (키 -> `Arc<Slot<T>>`)
    inflight: HashMap<String, Arc<dyn Any + Send + Sync>>,
    /// 볼륨별로 살아 있는 검사 스레드 수
    threads: HashMap<i64, usize>,
    timeouts: u64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct IoGuard {
    cfg: IoGuardConfig,
    reg: Mutex<Registry>,
}

impl IoGuard {
    pub fn new(cfg: IoGuardConfig) -> Arc<Self> {
        Arc::new(Self {
            cfg: IoGuardConfig {
                max_per_volume: cfg.max_per_volume.max(1),
                ..cfg
            },
            reg: Mutex::new(Registry::default()),
        })
    }

    pub fn timeout(&self) -> Duration {
        self.cfg.timeout
    }

    /// 볼륨이 응답 없음 상태인가 — 살아 있는(막힌) 검사 스레드가 상한에 닿았다.
    pub fn is_unresponsive(&self, volume_id: i64) -> bool {
        lock(&self.reg)
            .threads
            .get(&volume_id)
            .is_some_and(|n| *n >= self.cfg.max_per_volume)
    }

    /// 응답 없음 상태인 볼륨 (오름차순)
    pub fn unresponsive_volumes(&self) -> Vec<i64> {
        let mut v: Vec<i64> = lock(&self.reg)
            .threads
            .iter()
            .filter(|(_, n)| **n >= self.cfg.max_per_volume)
            .map(|(id, _)| *id)
            .collect();
        v.sort_unstable();
        v
    }

    /// 볼륨에서 지금 살아 있는 검사 스레드 수 (막힌 것 포함)
    pub fn live_checks(&self, volume_id: i64) -> usize {
        lock(&self.reg)
            .threads
            .get(&volume_id)
            .copied()
            .unwrap_or(0)
    }

    /// 제한 시간을 넘긴(또는 상한 때문에 걸지 않은) 검사 누계 (진단용)
    pub fn timeout_count(&self) -> u64 {
        lock(&self.reg).timeouts
    }

    /// 설정된 제한 시간으로 [`Self::guarded`] 를 부른다.
    pub fn run<T, F>(self: &Arc<Self>, key: &str, volume_id: i64, f: F) -> Guarded<T>
    where
        T: Clone + Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.guarded(key, volume_id, self.cfg.timeout, f)
    }

    /// `f`(블로킹 파일 호출)를 작업 스레드에서 돌리고 `timeout` 안에 끝나면 결과를 돌려준다.
    /// 같은 `key` 의 검사가 진행 중이면 `f` 는 버려지고 그 검사의 결과를 기다린다 (같은 키는 같은 결과 타입이어야 한다).
    pub fn guarded<T, F>(
        self: &Arc<Self>,
        key: &str,
        volume_id: i64,
        timeout: Duration,
        f: F,
    ) -> Guarded<T>
    where
        T: Clone + Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (slot, spawn) = {
            let mut reg = lock(&self.reg);
            if let Some(existing) = reg.inflight.get(key) {
                match Arc::clone(existing).downcast::<Slot<T>>() {
                    Ok(slot) => (slot, false),
                    Err(_) => return Self::timed_out(&mut reg),
                }
            } else {
                let live = reg.threads.entry(volume_id).or_insert(0);
                if *live >= self.cfg.max_per_volume {
                    return Self::timed_out(&mut reg);
                }
                *live += 1;
                let slot = Arc::new(Slot {
                    state: Mutex::new(SlotState::<T>::Running),
                    cv: Condvar::new(),
                });
                reg.inflight.insert(
                    key.to_owned(),
                    Arc::clone(&slot) as Arc<dyn Any + Send + Sync>,
                );
                (slot, true)
            }
        };
        if spawn && !self.spawn_worker(key, volume_id, &slot, f) {
            return self.timed_out_locked();
        }
        let st = lock(&slot.state);
        let (st, _) = slot
            .cv
            .wait_timeout_while(st, timeout, |s| matches!(s, SlotState::Running))
            .unwrap_or_else(|p| p.into_inner());
        if let SlotState::Done(v) = &*st {
            return Guarded::Done(v.clone());
        }
        // 제한 시간을 넘겼다 (작업 스레드가 패닉한 경우도 같다)
        drop(st);
        self.timed_out_locked()
    }

    fn timed_out<T>(reg: &mut Registry) -> Guarded<T> {
        reg.timeouts += 1;
        Guarded::TimedOut
    }

    fn timed_out_locked<T>(&self) -> Guarded<T> {
        Self::timed_out(&mut lock(&self.reg))
    }

    /// 작업 스레드를 시작한다. 시작하지 못하면 예약을 되돌리고 false.
    fn spawn_worker<T, F>(
        self: &Arc<Self>,
        key: &str,
        volume_id: i64,
        slot: &Arc<Slot<T>>,
        f: F,
    ) -> bool
    where
        T: Clone + Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (guard, key_owned, worker_slot) = (Arc::clone(self), key.to_owned(), Arc::clone(slot));
        let spawned = std::thread::Builder::new()
            .name("mam-io-guard".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(f));
                // 자리 반납이 결과 통지보다 먼저다 — 호출자는 통지를 받자마자 다음 검사를 걸 수 있는데,
                // 반납이 늦으면 상한에 걸려 정상 파일이 "응답 없음" 이 된다. 기다리던 호출자는 `slot` Arc 를 쥐고 있어
                // 키가 빠져도 결과를 받는다 (이 뒤의 같은 키 호출은 끝난 검사에 합류하는 대신 새로 검사한다)
                guard.release(&key_owned, volume_id);
                *lock(&worker_slot.state) = match result {
                    Ok(v) => SlotState::Done(v),
                    Err(_) => SlotState::Failed,
                };
                worker_slot.cv.notify_all();
            });
        if spawned.is_err() {
            self.release(key, volume_id);
            *lock(&slot.state) = SlotState::Failed;
            return false;
        }
        true
    }

    /// 검사가 끝났다 — 진행 중 표시를 지우고 볼륨의 살아 있는 스레드 수를 내린다.
    fn release(&self, key: &str, volume_id: i64) {
        let mut reg = lock(&self.reg);
        reg.inflight.remove(key);
        if let Some(n) = reg.threads.get_mut(&volume_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                reg.threads.remove(&volume_id);
            }
        }
    }
}
