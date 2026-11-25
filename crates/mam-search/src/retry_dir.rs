//! Windows 일시 접근 거부(`os error 5`/32/33)를 흡수하는 Tantivy `Directory` 래퍼.
//!
//! 새 세그먼트 파일 생성·`meta.json` 교체 등은 백신(Defender)·검색 인덱서·다른 핸들이 막 만든
//! 파일을 잠시 잡고 있으면 `ACCESS_DENIED`/공유 위반으로 간헐 실패한다. 잠시 뒤 다시 하면
//! 성공하므로 **짧은 지수 백오프로 한정 횟수만** 재시도하고, 그래도 실패하면 오류를 그대로
//! 올려 기존 commit 실패 계약(rollback + 배치 재적용)을 따른다.
//!
//! 재시도 대상은 멱등한 연산뿐이다: `open_write`(`create_new` 가 실패했으면 파일이 없다),
//! `atomic_write`, `atomic_read`, `open_read`/`get_file_handle`. **`delete` 는 재시도하지 않는다** —
//! reader 가 mmap 으로 잡은 세그먼트의 삭제 거부는 정상 상황이며 Tantivy 가 다음 GC 에서 다시
//! 시도하므로, 여기서 기다리면 commit 만 느려진다.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    Directory, DirectoryLock, FileHandle, FileSlice, Lock, WatchCallback, WatchHandle, WritePtr,
};

/// 재시도 한도. 최악의 총 대기 = 백오프 합 (기본값 ≈ 0.7초).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 첫 시도를 포함한 총 시도 횟수 (1 이면 재시도 없음).
    pub max_attempts: u32,
    /// 첫 재시도 전 대기. 이후 매번 2배.
    pub initial_backoff: Duration,
    /// 한 번의 대기 상한.
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 8,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(200),
        }
    }
}

impl RetryPolicy {
    /// 대기 없이 `max_attempts` 번 시도 — 테스트용.
    pub fn immediate(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            initial_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }

    fn backoff(&self, retry_index: u32) -> Duration {
        let factor = 1u32.checked_shl(retry_index).unwrap_or(u32::MAX);
        self.initial_backoff
            .saturating_mul(factor)
            .min(self.max_backoff)
    }
}

/// 일시적일 수 있는 접근 거부인가. Windows 에서는 공유 위반(32)·잠금 위반(33)도 포함한다.
pub fn is_transient(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::PermissionDenied {
        return true;
    }
    cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33))
}

/// 멱등한 파일 연산 `op` 를 일시 접근 거부에 한해 백오프로 재시도한다 (Directory 밖의 파일 연산용).
pub fn retry_io<T>(policy: RetryPolicy, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempt = 0u32;
    loop {
        match op() {
            Err(e) if is_transient(&e) && attempt + 1 < policy.max_attempts => {
                std::thread::sleep(policy.backoff(attempt));
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// 내부 `Directory` 의 일시 오류를 재시도하는 래퍼.
#[derive(Clone, Debug)]
pub struct RetryDirectory {
    inner: Box<dyn Directory>,
    policy: RetryPolicy,
    retries: Arc<AtomicU64>,
}

impl RetryDirectory {
    pub fn new(inner: Box<dyn Directory>, policy: RetryPolicy) -> Self {
        Self {
            inner,
            policy,
            retries: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 지금까지 수행한 재시도(= 흡수한 일시 오류) 횟수.
    pub fn retries(&self) -> u64 {
        self.retries.load(Ordering::Relaxed)
    }

    fn run<T, E>(
        &self,
        mut op: impl FnMut() -> Result<T, E>,
        transient: impl Fn(&E) -> bool,
    ) -> Result<T, E> {
        let mut attempt = 0u32;
        loop {
            match op() {
                Err(e) if transient(&e) && attempt + 1 < self.policy.max_attempts => {
                    std::thread::sleep(self.policy.backoff(attempt));
                    attempt += 1;
                    self.retries.fetch_add(1, Ordering::Relaxed);
                }
                other => return other,
            }
        }
    }
}

fn read_transient(e: &OpenReadError) -> bool {
    matches!(e, OpenReadError::IoError { io_error, .. } if is_transient(io_error))
}

fn write_transient(e: &OpenWriteError) -> bool {
    matches!(e, OpenWriteError::IoError { io_error, .. } if is_transient(io_error))
}

impl Directory for RetryDirectory {
    fn get_file_handle(&self, p: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.run(|| self.inner.get_file_handle(p), read_transient)
    }
    fn open_read(&self, p: &Path) -> Result<FileSlice, OpenReadError> {
        self.run(|| self.inner.open_read(p), read_transient)
    }
    fn delete(&self, p: &Path) -> Result<(), DeleteError> {
        self.inner.delete(p)
    }
    fn exists(&self, p: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(p)
    }
    fn open_write(&self, p: &Path) -> Result<WritePtr, OpenWriteError> {
        self.run(|| self.inner.open_write(p), write_transient)
    }
    fn atomic_read(&self, p: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.run(|| self.inner.atomic_read(p), read_transient)
    }
    fn atomic_write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        self.run(|| self.inner.atomic_write(p, data), is_transient)
    }
    fn sync_directory(&self) -> io::Result<()> {
        self.inner.sync_directory()
    }
    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        self.inner.acquire_lock(lock)
    }
    fn watch(&self, cb: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.inner.watch(cb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_fts_22_backoff_doubles_and_is_capped() {
        let p = RetryPolicy {
            max_attempts: 8,
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(200),
        };
        let ms: Vec<u128> = (0..7).map(|i| p.backoff(i).as_millis()).collect();
        assert_eq!(ms, [10, 20, 40, 80, 160, 200, 200]);
        assert_eq!(p.backoff(99), Duration::from_millis(200)); // 시프트 오버플로 없음
    }

    #[test]
    fn t_fts_22b_transient_classification() {
        assert!(is_transient(&io::Error::from(
            io::ErrorKind::PermissionDenied
        )));
        assert!(!is_transient(&io::Error::from(io::ErrorKind::NotFound)));
        assert!(!is_transient(&io::Error::from(
            io::ErrorKind::AlreadyExists
        )));
    }
}
