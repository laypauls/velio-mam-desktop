//! 호스트 프로세스가 죽으면 워커도 죽게 한다 — Windows Job Object(`KILL_ON_JOB_CLOSE`).
//!
//! 워커를 띄운 직후 익명 Job 에 넣고 핸들을 `WorkerProc` 이 쥔다. 호스트가 정상 종료·크래시·강제 종료되면 OS 가 핸들을 닫고,
//! 그 순간 Job 의 남은 프로세스가 죽는다(JS 무한 루프 중인 워커가 CPU 한도까지 잔류하지 않는다). 비 Windows 는 아무것도 하지 않는다.
//! Job 생성·할당이 실패하면(예: 중첩을 막는 Job 안에서 실행) 경고를 한 번 남기고 `None` — 워커는 그대로 돌고 평소 종료 경로(타임아웃·kill)가 지킨다.
//! 패닉하지 않는다. 종료는 PID 가 아닌 Job 핸들로만 이뤄진다(이름으로 죽이지 않는다).

//!
//! 같은 Job 에 방어 한도도 건다 — 활성 프로세스 1개(워커가 자식을 못 만든다), 프로세스 커밋 메모리 상한(`process_memory_cap`),
//! 처리되지 않은 예외 시 오류 대화상자 없이 즉시 종료(`DIE_ON_UNHANDLED_EXCEPTION`). 낮은 무결성 토큰·AppContainer 는 후속 버전으로 미룬다.

use std::process::Child;

/// 프로세스 커밋 메모리 하한 — QuickJS 힙(기본 64 MiB) + 줄 버퍼(최대 4 MiB 줄이 문자열·Value 로 여러 벌) + 바이너리·스택·할당기 여유
const MIN_PROCESS_MEMORY: u64 = 256 << 20;

/// 워커 프로세스 1개의 메모리 상한(바이트). QuickJS 한도(`memory_bytes`)보다 항상 커서 평소엔 QuickJS 가 먼저 `limit` 으로 끊고,
/// 엔진 밖(줄 버퍼·파서) 폭주나 QuickJS 한도 해제(0)일 때만 OS 가 워커를 끊는다. 한도가 크게 설정돼도 2 배 + 64 MiB 로 따라간다
pub(crate) fn process_memory_cap(quickjs_bytes: u64) -> u64 {
    quickjs_bytes.saturating_mul(2).saturating_add(64 << 20).max(MIN_PROCESS_MEMORY)
}

/// 열려 있는 동안 워커를 보호한다. `Drop` 시 Job 이 닫혀 남은 멤버가 죽는다(Windows).
pub(crate) struct KillOnClose {
    #[cfg(windows)]
    _job: win::Job,
}

#[cfg(all(test, windows))]
impl KillOnClose {
    fn applied(&self) -> Option<win::AppliedLimits> {
        self._job.applied()
    }
}

impl KillOnClose {
    /// 방금 띄운 `child` 를 Job 에 넣는다. 실패하면 경고 후 `None`
    pub(crate) fn attach(child: &Child, memory_cap: u64) -> Option<Self> {
        #[cfg(windows)]
        {
            match win::Job::new(memory_cap).and_then(|j| j.assign(child).then_some(j)) {
                Some(job) => Some(Self { _job: job }),
                None => {
                    warn_once("job object not attached; worker relies on normal kill paths");
                    None
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (child, memory_cap);
            None
        }
    }
}

#[cfg(windows)]
fn warn_once(msg: &str) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| eprintln!("mam-plugin-host: {msg}"));
}

#[cfg(windows)]
mod win {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };

    #[cfg(test)]
    use windows_sys::Win32::System::JobObjects::QueryInformationJobObject;

    /// 시험용 — Job 에 실제로 걸린 한도: (LimitFlags, ActiveProcessLimit, ProcessMemoryLimit)
    #[cfg(test)]
    pub(super) type AppliedLimits = (u32, u32, usize);

    pub(super) struct Job(HANDLE);

    // SAFETY: Job 핸들은 스레드 간 이동·공유가 안전한 커널 객체 핸들이다.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub(super) fn new(memory_cap: u64) -> Option<Self> {
            // SAFETY: 널 속성·이름으로 익명 Job 을 만든다. 반환 핸들은 Job 이 소유해 Drop 에서 한 번 닫는다.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return None;
                }
                let job = Job(h);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
                info.BasicLimitInformation.ActiveProcessLimit = 1;
                info.ProcessMemoryLimit = usize::try_from(memory_cap).unwrap_or(usize::MAX);
                let ok = SetInformationJobObject(h, JobObjectExtendedLimitInformation, (&raw const info).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
                (ok != 0).then_some(job) // 실패 시 Drop 이 핸들을 닫는다
            }
        }

        #[cfg(test)]
        pub(super) fn applied(&self) -> Option<AppliedLimits> {
            // SAFETY: 이 Job 의 유효한 핸들에 대해 크기가 맞는 버퍼로 조회한다.
            unsafe {
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                let ok = QueryInformationJobObject(self.0, JobObjectExtendedLimitInformation, (&raw mut info).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32, std::ptr::null_mut());
                (ok != 0).then_some((info.BasicLimitInformation.LimitFlags, info.BasicLimitInformation.ActiveProcessLimit, info.ProcessMemoryLimit))
            }
        }

        pub(super) fn assign(&self, child: &Child) -> bool {
            // SAFETY: child 핸들은 Child 가 살아 있는 동안 유효하다.
            unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) != 0 }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: 이 구조체가 소유한 핸들을 한 번만 닫는다. KILL_ON_JOB_CLOSE 라 남은 멤버가 죽는다.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};

    use super::{KillOnClose, process_memory_cap};

    const CAP: u64 = 256 << 20;

    /// T-phost-job-03 (Business rule): Job 에 kill-on-close·활성 프로세스 1·메모리 상한·예외 즉시 종료가 모두 걸린다
    #[test]
    fn t_phost_job_03_job_carries_the_defensive_limits() {
        use windows_sys::Win32::System::JobObjects::{
            JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        };
        let mut child = Command::new("ping.exe").args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().expect("spawn ping");
        let guard = KillOnClose::attach(&child, CAP).expect("attach");
        let (flags, active, mem) = guard.applied().expect("query job");
        let _ = child.kill();
        let _ = child.wait();
        let want = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        assert_eq!(flags & want, want);
        assert_eq!(active, 1);
        assert_eq!(mem as u64, CAP);
    }

    /// T-phost-job-04 (Boundary): 메모리 상한은 QuickJS 한도보다 항상 크고 하한 256 MiB 이며 포화 연산으로 넘치지 않는다
    #[test]
    fn t_phost_job_04_memory_cap_scales_above_the_quickjs_limit() {
        assert_eq!(process_memory_cap(0), 256 << 20);
        assert_eq!(process_memory_cap(64 << 20), 256 << 20);
        assert_eq!(process_memory_cap(512 << 20), (1024 + 64) << 20);
        assert!(process_memory_cap(u64::MAX) >= u64::MAX / 2);
    }

    /// T-phost-job-01 (Happy): 살아 있는 프로세스는 Job 에 들어간다
    #[test]
    fn t_phost_job_01_attach_succeeds_for_a_live_child() {
        let mut child = Command::new("ping.exe").args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().expect("spawn ping");
        let guard = KillOnClose::attach(&child, CAP);
        let attached = guard.is_some();
        let t = std::time::Instant::now();
        drop(guard); // KILL_ON_JOB_CLOSE — 닫으면 자식도 죽는다
        child.wait().expect("wait");
        assert!(attached, "job attach should succeed for a live child");
        // ping -n 60 은 정상이면 약 60 초 걸린다 — Job 이 닫히며 죽었다면 훨씬 빨리 끝난다
        assert!(t.elapsed() < std::time::Duration::from_secs(30), "child survived job close");
    }

    /// T-phost-job-02 (External failure): 이미 끝난 프로세스는 할당 실패 -> 패닉 없이 None
    #[test]
    fn t_phost_job_02_attach_to_exited_child_returns_none_without_panic() {
        let mut child = Command::new("cmd.exe").args(["/C", "exit 0"]).stdout(Stdio::null()).spawn().expect("spawn cmd");
        child.wait().expect("wait");
        assert!(KillOnClose::attach(&child, CAP).is_none());
    }
}
