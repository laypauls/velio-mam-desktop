//! 단일 인스턴스 잠금 — `MAMData/.lock` 에 배타적 파일 잠금을 건다.
//!
//! 프로세스가 죽으면 OS 가 잠금을 풀므로 오래된 잠금 파일이 남아도 다음 실행을 막지 않는다.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use super::StorageError;

/// 살아 있는 동안 잠금을 유지한다(Drop 시 해제).
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// 이미 다른 인스턴스(또는 같은 프로세스의 다른 핸들)가 잠그고 있으면 `AlreadyRunning`.
    pub fn acquire(lock_file: &Path) -> Result<Self, StorageError> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_file)
            .map_err(|e| StorageError::io(lock_file, e))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(StorageError::AlreadyRunning),
            Err(TryLockError::Error(e)) => Err(StorageError::io(lock_file, e)),
        }
    }
}
