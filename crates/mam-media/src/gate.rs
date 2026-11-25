//! 사이드카 게이트 — 특정 경로(볼륨 마운트 지점) 아래 파일을 쓰는 FFmpeg/FFprobe 실행을 막고, 실행 중인 것을 죽인다.
//!
//! USB 디스크 "꺼내기 준비" 가 쓴다: 사이드카는 자식 프로세스라 작업 스레드를 양보시키는 것만으로는 파일 핸들이 닫히지 않는다.
//! 실행기([`crate::CancellableProcessRunner`])가 프로세스를 띄우기 **전에** [`SidecarGate::enter`] 로 등록하고(차단 경로면 거부),
//! 실행 루프가 항목의 취소 플래그를 확인해 트리를 종료한다 (Job Object, 폴백은 PID 기준 `taskkill /T` — 이름으로 죽이지 않는다).
//! 경로 일치는 인자 문자열의 접두 비교다(대소문자 무시, `/`·`\` 동일 취급, `\\?\` 제거).

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 비교용 정규화: `\\?\` 제거, `/` -> `\`, 소문자.
fn normalize(s: &str) -> String {
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    s.replace('/', "\\").to_lowercase()
}

/// 디렉터리 접두(끝에 `\` 보장). 인자가 접두와 같거나 그 아래면 일치.
fn dir_prefix(p: &str) -> String {
    let mut n = normalize(p);
    if !n.ends_with('\\') {
        n.push('\\');
    }
    n
}

fn under(arg: &str, prefix: &str) -> bool {
    let a = normalize(arg);
    a.starts_with(prefix) || format!("{a}\\") == prefix
}

struct Entry {
    args: Vec<String>,
    cancel: Arc<AtomicBool>,
    /// 0 = 아직 띄우지 않음
    pid: Arc<AtomicU32>,
}

#[derive(Default)]
struct State {
    blocked: Vec<String>,
    running: HashMap<u64, Entry>,
    next: u64,
}

/// 경로 접두 기준 사이드카 차단·종료. [`crate::CancellableProcessRunner::with_gate`] 로 실행기에 연결한다.
#[derive(Default)]
pub struct SidecarGate {
    state: Mutex<State>,
    cv: Condvar,
}

impl std::fmt::Debug for SidecarGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = guard(&self.state);
        f.debug_struct("SidecarGate")
            .field("blocked", &g.blocked)
            .field("running", &g.running.len())
            .finish()
    }
}

fn guard(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl SidecarGate {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// `prefix` 아래 파일을 인자로 받는 새 실행을 거부하기 시작한다. 이미 실행 중인 것은 [`Self::cancel_under`] 로 죽인다.
    pub fn block(&self, prefix: &str) {
        let p = dir_prefix(prefix);
        let mut g = guard(&self.state);
        if !g.blocked.contains(&p) {
            g.blocked.push(p);
        }
    }

    pub fn unblock(&self, prefix: &str) {
        let p = dir_prefix(prefix);
        guard(&self.state).blocked.retain(|b| *b != p);
    }

    /// `prefix` 아래 파일을 쓰는 실행 중인 사이드카에 종료를 요청한다. 이미 띄운 프로세스의 PID 를 돌려준다
    /// (실제 트리 종료는 각 실행 루프가 수 ms 안에 한다).
    pub fn cancel_under(&self, prefix: &str) -> Vec<u32> {
        let p = dir_prefix(prefix);
        let g = guard(&self.state);
        let mut pids = Vec::new();
        for e in g
            .running
            .values()
            .filter(|e| e.args.iter().any(|a| under(a, &p)))
        {
            e.cancel.store(true, Ordering::SeqCst);
            let pid = e.pid.load(Ordering::SeqCst);
            if pid != 0 {
                pids.push(pid);
            }
        }
        pids
    }

    /// `prefix` 아래 파일을 쓰는 실행 중인 사이드카 수.
    pub fn running_under(&self, prefix: &str) -> usize {
        let p = dir_prefix(prefix);
        guard(&self.state)
            .running
            .values()
            .filter(|e| e.args.iter().any(|a| under(a, &p)))
            .count()
    }

    /// `prefix` 아래를 쓰는 사이드카가 모두 끝날 때까지 기다린다. 끝났으면 true, 시간 초과면 false.
    pub fn wait_none_under(&self, prefix: &str, timeout: Duration) -> bool {
        let p = dir_prefix(prefix);
        let deadline = Instant::now() + timeout;
        let mut g = guard(&self.state);
        loop {
            if !g
                .running
                .values()
                .any(|e| e.args.iter().any(|a| under(a, &p)))
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// 실행 직전 등록. 차단된 경로를 쓰면 `None`. 반환 핸들이 살아 있는 동안 "실행 중" 이다.
    pub(crate) fn enter(self: &Arc<Self>, args: &[OsString]) -> Option<Entered> {
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let mut g = guard(&self.state);
        if g.blocked.iter().any(|b| args.iter().any(|a| under(a, b))) {
            return None;
        }
        let id = g.next;
        g.next += 1;
        let (cancel, pid) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU32::new(0)),
        );
        g.running.insert(
            id,
            Entry {
                args,
                cancel: Arc::clone(&cancel),
                pid: Arc::clone(&pid),
            },
        );
        Some(Entered {
            gate: Arc::clone(self),
            id,
            cancel,
            pid,
        })
    }
}

/// 등록된 실행 1건 — drop 하면 등록을 지우고 기다리는 쪽을 깨운다.
pub(crate) struct Entered {
    gate: Arc<SidecarGate>,
    id: u64,
    cancel: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
}

impl Entered {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub(crate) fn set_pid(&self, pid: u32) {
        self.pid.store(pid, Ordering::SeqCst);
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        guard(&self.gate.state).running.remove(&self.id);
        self.gate.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary (접두 경계·대소문자·구분자) / Concurrency (대기 해제)
    use std::ffi::OsString;

    use super::*;

    fn a(s: &str) -> Vec<OsString> {
        vec![OsString::from("-i"), OsString::from(s)]
    }

    #[test]
    fn t_gate_01_blocks_only_paths_under_the_prefix_case_and_separator_insensitive() {
        // Arrange
        let gate = SidecarGate::new();
        gate.block(r"E:\");

        // Act / Assert
        assert!(gate.enter(&a(r"E:\data\a.mp4")).is_none());
        assert!(gate.enter(&a("e:/Data/a.mp4")).is_none());
        assert!(gate.enter(&a(r"\\?\E:\data\a.mp4")).is_none());
        assert!(gate.enter(&a(r"D:\data\a.mp4")).is_some());
        assert!(
            gate.enter(&a(r"E2:\x")).is_some(),
            "other drive prefix is not a match"
        );
        gate.unblock(r"E:\");
        assert!(gate.enter(&a(r"E:\data\a.mp4")).is_some());
    }

    #[test]
    fn t_gate_02_cancel_under_flags_only_matching_running_entries_and_reports_pids() {
        // Arrange
        let gate = SidecarGate::new();
        let on_e = gate.enter(&a(r"E:\x.mp4")).unwrap();
        let on_d = gate.enter(&a(r"D:\x.mp4")).unwrap();
        on_e.set_pid(4242);

        // Act
        let pids = gate.cancel_under(r"E:\");

        // Assert
        assert_eq!(pids, vec![4242]);
        assert!(on_e.is_cancelled());
        assert!(!on_d.is_cancelled());
        assert_eq!(gate.running_under(r"E:\"), 1);
    }

    #[test]
    fn t_gate_03_wait_none_under_returns_after_the_entry_drops_and_times_out_otherwise() {
        // Arrange
        let gate = SidecarGate::new();
        let held = gate.enter(&a(r"E:\x.mp4")).unwrap();

        // Act / Assert: 시간 초과
        assert!(!gate.wait_none_under(r"E:\", Duration::from_millis(30)));
        // 다른 스레드가 놓으면 깨어난다
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            drop(held);
        });
        assert!(gate.wait_none_under(r"E:\", Duration::from_secs(10)));
        t.join().unwrap();
    }
}
