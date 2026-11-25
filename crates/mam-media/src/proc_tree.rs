//! 사이드카 프로세스 트리 종료 — Windows Job Object.
//!
//! PATH 의 ffmpeg 가 shim(Chocolatey 등)이면 shim 이 진짜 ffmpeg 를 **자식**으로 띄운다. 직접 띄운 프로세스만
//! `kill()` 하면 손자가 고아로 남아 CPU 를 쓰고 stdout 파이프를 쥔 채 리더를 막는다. 그래서 자식을
//! `KILL_ON_JOB_CLOSE` Job 에 넣고, 종료 시 `TerminateJobObject` 로 트리 전체를 죽인다.
//!
//! 경쟁 처리: 자식을 `CREATE_SUSPENDED` 로 띄워 Job 에 넣은 **뒤** 주 스레드를 재개한다 — shim 이 손자를
//! 만들기 전에 Job 소속이 확정되므로 놓치는 구간이 없다. Job 생성·할당이 실패하면(예: 앱이 중첩을 막는 Job 안에서
//! 실행) 경고를 한 번 남기고 `taskkill /T` 로 트리를 최선으로 죽이는 폴백으로 내려간다. 패닉하지 않는다.

use std::process::Child;

/// 프로세스 트리 종료 핸들. `Drop` 시 Job 이 닫히며 남은 프로세스도 죽는다 (Windows).
pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: Option<win::Job>,
    #[cfg(windows)]
    attached: bool,
}

impl ProcessTree {
    /// 띄우기 전에 만든다. Windows 에서 `suspended()` 가 true 면 `CREATE_SUSPENDED` 로 띄워야 한다.
    pub(crate) fn new() -> Self {
        #[cfg(windows)]
        {
            Self {
                job: win::Job::new(),
                attached: false,
            }
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }

    /// `CREATE_SUSPENDED` 로 띄워야 하는가 (Job 을 만든 경우).
    pub(crate) fn suspended(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// 띄운 직후 호출: Job 에 할당하고(가능하면) 주 스레드를 재개한다. 재개 실패 시 자식을 죽이고 오류를 돌려준다.
    pub(crate) fn adopt(&mut self, child: &Child) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                self.attached = job.assign(child);
                if !self.attached {
                    warn_once("job assign failed; falling back to taskkill /T");
                }
                return win::resume_process_threads(child.id());
            }
        }
        let _ = child;
        Ok(())
    }

    /// 트리 전체를 종료한다 (여러 번 호출해도 안전).
    pub(crate) fn terminate(&self, child: &mut Child) {
        #[cfg(windows)]
        {
            if self.attached
                && let Some(job) = &self.job
            {
                job.terminate();
            } else if matches!(child.try_wait(), Ok(None)) {
                // 폴백: 이미 끝난 프로세스에는 taskkill 을 띄우지 않는다.
                win::taskkill_tree(child.id());
            }
        }
        let _ = child.kill();
    }
}

#[cfg(windows)]
fn warn_once(msg: &str) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| eprintln!("mam-media: {msg}"));
}

#[cfg(windows)]
mod win {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::{Child, Command, Stdio};

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    pub(super) struct Job(HANDLE);

    impl Job {
        pub(super) fn new() -> Option<Self> {
            // SAFETY: 널 속성·이름으로 익명 Job 생성. 반환 핸들은 Job 이 소유해 Drop 에서 닫는다.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    super::warn_once("CreateJobObjectW failed; falling back to taskkill /T");
                    return None;
                }
                let job = Job(h);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    super::warn_once("SetInformationJobObject failed; falling back to taskkill /T");
                    return None; // Drop 이 핸들을 닫는다
                }
                Some(job)
            }
        }

        pub(super) fn assign(&self, child: &Child) -> bool {
            // SAFETY: child 핸들은 Child 가 살아 있는 동안 유효하다.
            unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) != 0 }
        }

        pub(super) fn terminate(&self) {
            // SAFETY: 유효한 Job 핸들.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // KILL_ON_JOB_CLOSE: 닫으면 남은 멤버가 죽는다.
            // SAFETY: 이 구조체가 소유한 핸들을 한 번만 닫는다.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// `CREATE_SUSPENDED` 로 띄운 프로세스의 (유일한) 초기 스레드를 재개한다.
    pub(super) fn resume_process_threads(pid: u32) -> std::io::Result<()> {
        // SAFETY: ToolHelp 스냅샷을 열거하고 핸들을 모두 닫는다.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut resumed = false;
            let mut more = Thread32First(snap, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    let th = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !th.is_null() {
                        if ResumeThread(th) != u32::MAX {
                            resumed = true;
                        }
                        CloseHandle(th);
                    }
                }
                more = Thread32Next(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            if resumed {
                Ok(())
            } else {
                Err(std::io::Error::other("could not resume suspended sidecar"))
            }
        }
    }

    /// Job 을 못 쓸 때의 폴백. 이름이 아니라 PID 로만 죽인다.
    pub(super) fn taskkill_tree(pid: u32) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }
}
