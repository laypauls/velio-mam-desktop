//! 사용자 세션 단위 단일 인스턴스 — Windows 명명 뮤텍스 `Local\<식별자>.instance`.
//!
//! `.lock`(`InstanceLock`)은 데이터 위치별이라 `MAM_DATA_DIR` 이 다르면 두 번째 인스턴스가 뜬다.
//! 이 뮤텍스는 데이터 위치와 무관하게 같은 Windows 세션의 같은 앱 식별자를 하나로 묶는다.
//!
//! 소유권을 기다리지 않으므로 `WAIT_ABANDONED` 는 생기지 않는다 — 핸들만 쥐고 있고, 프로세스가 죽으면
//! 커널이 핸들을 닫아 객체가 사라진다. 이미 있다는 신호는 `ERROR_ALREADY_EXISTS` 다.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE,
};
use windows_sys::Win32::System::Threading::CreateMutexW;

use super::StorageError;

/// 살아 있는 동안 뮤텍스 핸들을 유지한다(Drop 시 해제).
#[derive(Debug)]
pub struct InstanceMutex {
    handle: HANDLE,
}

// 핸들은 소유권만 나타내는 커널 객체 참조이고 이 타입은 핸들로 다른 호출을 하지 않는다.
unsafe impl Send for InstanceMutex {}
unsafe impl Sync for InstanceMutex {}

impl InstanceMutex {
    /// `Local\<identifier>.instance` 를 만든다. 다른 인스턴스가 이미 갖고 있으면 `AlreadyRunning`.
    pub fn acquire(identifier: &str) -> Result<Self, StorageError> {
        let name = mutex_name(identifier)?;
        let wide: Vec<u16> = std::ffi::OsStr::new(&name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: `wide` 는 NUL 로 끝나는 유효한 UTF-16 문자열이고 호출 동안 살아 있다. 보안 속성은 기본값(null).
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
        // SAFETY: 직전 호출의 스레드별 오류 값을 읽는다 (사이에 다른 Win32 호출 없음).
        let last = unsafe { GetLastError() };
        if handle.is_null() {
            // 같은 이름의 객체가 있는데 접근 권한이 없으면 다른 주체의 인스턴스다
            if last == ERROR_ACCESS_DENIED {
                return Err(StorageError::AlreadyRunning);
            }
            return Err(StorageError::io(
                Path::new(&name),
                io::Error::from_raw_os_error(last as i32),
            ));
        }
        if last == ERROR_ALREADY_EXISTS {
            // 기존 객체의 핸들을 얻었을 뿐 소유자는 아니다 — 닫고 알린다
            // SAFETY: 위에서 얻은 유효한 핸들을 한 번만 닫는다.
            unsafe { CloseHandle(handle) };
            return Err(StorageError::AlreadyRunning);
        }
        Ok(Self { handle })
    }
}

impl Drop for InstanceMutex {
    fn drop(&mut self) {
        // SAFETY: `acquire` 가 얻은 유효한 핸들을 한 번만 닫는다.
        unsafe { CloseHandle(self.handle) };
    }
}

fn mutex_name(identifier: &str) -> Result<String, StorageError> {
    // `\` 는 네임스페이스 구분자, NUL 은 문자열 끝 — 식별자에 있으면 의도와 다른 객체가 된다
    if identifier.is_empty() || identifier.contains(['\\', '\0']) {
        return Err(StorageError::io(
            Path::new(identifier),
            io::Error::new(io::ErrorKind::InvalidInput, "invalid app identifier"),
        ));
    }
    Ok(format!("Local\\{identifier}.instance"))
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Validation / Concurrency

    use super::*;

    fn unique(tag: &str) -> String {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("rng");
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        format!("com.mam.test.{tag}.{hex}")
    }

    #[test]
    fn t_imutex_01_second_acquire_same_name_fails_while_held() {
        // Arrange
        let id = unique("a");
        let first = InstanceMutex::acquire(&id);
        // Act
        let second = InstanceMutex::acquire(&id);
        // Assert
        assert!(first.is_ok());
        assert!(matches!(second, Err(StorageError::AlreadyRunning)));
    }

    #[test]
    fn t_imutex_02_different_name_succeeds() {
        // Arrange
        let (a, b) = (unique("b1"), unique("b2"));
        // Act
        let first = InstanceMutex::acquire(&a);
        let second = InstanceMutex::acquire(&b);
        // Assert
        assert!(first.is_ok() && second.is_ok());
    }

    #[test]
    fn t_imutex_03_reacquire_after_drop() {
        // Arrange
        let id = unique("c");
        let first = InstanceMutex::acquire(&id).expect("first");
        // Act
        drop(first);
        let again = InstanceMutex::acquire(&id);
        // Assert
        assert!(again.is_ok(), "mutex must be released on drop");
    }

    #[test]
    fn t_imutex_04_invalid_identifier_rejected() {
        // Arrange / Act / Assert
        for bad in ["", "a\\b", "a\0b"] {
            assert!(
                matches!(InstanceMutex::acquire(bad), Err(StorageError::Io { .. })),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn t_imutex_05_only_one_of_many_threads_wins() {
        // Arrange
        let id = unique("d");
        // Act
        let results: Vec<_> = (0..8)
            .map(|_| {
                let id = id.clone();
                std::thread::spawn(move || {
                    let r = InstanceMutex::acquire(&id);
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    r.is_ok()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect();
        // Assert
        assert_eq!(results.iter().filter(|ok| **ok).count(), 1);
    }
}
