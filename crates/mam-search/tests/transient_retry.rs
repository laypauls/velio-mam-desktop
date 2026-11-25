//! Windows 일시 접근 거부(os error 5) 재시도 테스트 — `Directory` 래퍼로 실패를 결정적으로 주입한다.
//! 분류: T-fts-23 External failure(일시 → 흡수) / T-fts-23b External failure(지속 → 오류+복구) /
//!       T-fts-23c Boundary(비일시 오류는 재시도 안 함) / T-fts-23d atomic_write / T-fts-23e meta 생성

mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use mam_search::FilenameStrategy::A;
use mam_search::{IndexOptions, RetryPolicy, SearchError, SearchIndex};
use tantivy::directory::FileSlice;
use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    Directory, DirectoryLock, FileHandle, Lock, MmapDirectory, WatchCallback, WatchHandle, WritePtr,
};
use tempfile::tempdir;

#[derive(Clone, Debug, Default)]
struct Faults {
    /// 데이터 파일 `open_write` 를 앞으로 이만큼 실패시킨다.
    open_write: Arc<AtomicUsize>,
    /// `meta.json` `atomic_write` 를 앞으로 이만큼 실패시킨다.
    atomic_write: Arc<AtomicUsize>,
    /// 주입할 오류 종류 (기본 PermissionDenied).
    other_kind: Arc<AtomicUsize>,
    /// 데이터 파일 `open_write` 호출 총수 (시도 횟수 검증).
    open_write_calls: Arc<AtomicUsize>,
}

#[derive(Clone, Debug)]
struct FlakyDir {
    inner: MmapDirectory,
    f: Faults,
}

fn take(n: &AtomicUsize) -> bool {
    n.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
        .is_ok()
}

impl FlakyDir {
    fn err(&self) -> io::Error {
        if self.f.other_kind.load(Ordering::SeqCst) != 0 {
            io::Error::from(io::ErrorKind::InvalidData)
        } else {
            io::Error::from(io::ErrorKind::PermissionDenied)
        }
    }
}

impl Directory for FlakyDir {
    fn get_file_handle(&self, p: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        self.inner.get_file_handle(p)
    }
    fn open_read(&self, p: &Path) -> Result<FileSlice, OpenReadError> {
        self.inner.open_read(p)
    }
    fn delete(&self, p: &Path) -> Result<(), DeleteError> {
        self.inner.delete(p)
    }
    fn exists(&self, p: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(p)
    }
    fn open_write(&self, p: &Path) -> Result<WritePtr, OpenWriteError> {
        let data = matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("idx" | "store" | "fast")
        );
        if data {
            self.f.open_write_calls.fetch_add(1, Ordering::SeqCst);
            if take(&self.f.open_write) {
                return Err(OpenWriteError::wrap_io_error(self.err(), PathBuf::from(p)));
            }
        }
        self.inner.open_write(p)
    }
    fn atomic_read(&self, p: &Path) -> Result<Vec<u8>, OpenReadError> {
        self.inner.atomic_read(p)
    }
    fn atomic_write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        if p.ends_with("meta.json") && take(&self.f.atomic_write) {
            return Err(self.err());
        }
        self.inner.atomic_write(p, data)
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

fn open(dir: &tempfile::TempDir, attempts: u32) -> (SearchIndex, Faults) {
    let faults = Faults::default();
    let f = faults.clone();
    let idx = SearchIndex::open_or_create_with_retry(
        &dir.path().join("idx"),
        small(IndexOptions::all()),
        RetryPolicy::immediate(attempts),
        move |inner| Box::new(FlakyDir { inner, f }),
    )
    .unwrap();
    (idx, faults)
}

fn fill(idx: &SearchIndex, n: u64) {
    for i in 0..n {
        idx.upsert(&doc(i, &format!("img{i}.jpg"))).unwrap();
    }
}

#[test]
fn t_fts_23_transient_open_write_denials_are_absorbed() {
    // Arrange: 데이터 파일 생성이 3번 접근 거부 후 성공하도록 한다 (시도 한도 8)
    let dir = tempdir().unwrap();
    let (idx, f) = open(&dir, 8);
    fill(&idx, 20);
    f.open_write.store(3, Ordering::SeqCst);
    // Act
    idx.commit().expect("transient denials must be retried");
    // Assert: 주입이 모두 소비됐고 문서는 손실·중복 없이 반영
    assert_eq!(f.open_write.load(Ordering::SeqCst), 0);
    assert_eq!(idx.num_docs(), 20);
    assert_eq!(ids(&idx, &req("img7", A)), [7]);
}

#[test]
fn t_fts_23b_persistent_denial_exhausts_attempts_then_errors_and_recovers() {
    // Arrange: 한도 3, 실패 100회 → 첫 파일에서 정확히 3번 시도 후 포기
    let dir = tempdir().unwrap();
    let (idx, f) = open(&dir, 3);
    fill(&idx, 10);
    f.open_write.store(100, Ordering::SeqCst);
    // Act
    let err = idx
        .commit()
        .expect_err("must fail after attempts exhausted");
    // Assert: Tantivy 오류, 시도 횟수 = 한도, 배치는 버려짐(기존 rollback 계약)
    assert!(matches!(err, SearchError::Tantivy(_)), "{err:?}");
    assert_eq!(f.open_write_calls.load(Ordering::SeqCst), 3);
    assert_eq!(idx.num_docs(), 0);
    // 원인이 사라지면 배치를 다시 적용해 복구된다
    f.open_write.store(0, Ordering::SeqCst);
    fill(&idx, 10);
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 10);
}

#[test]
fn t_fts_23c_non_transient_error_is_not_retried() {
    // Arrange: PermissionDenied 가 아닌 오류 1회 주입
    let dir = tempdir().unwrap();
    let (idx, f) = open(&dir, 8);
    fill(&idx, 5);
    f.other_kind.store(1, Ordering::SeqCst);
    f.open_write.store(1, Ordering::SeqCst);
    let before = f.open_write_calls.load(Ordering::SeqCst);
    // Act
    let err = idx.commit().expect_err("non-transient error must surface");
    // Assert: 재시도 없이 첫 호출에서 실패 (그 뒤 호출은 rollback 이후 없음)
    assert!(matches!(err, SearchError::Tantivy(_)), "{err:?}");
    assert_eq!(f.open_write_calls.load(Ordering::SeqCst) - before, 1);
}

#[test]
fn t_fts_23d_transient_meta_json_replace_is_absorbed() {
    let dir = tempdir().unwrap();
    let (idx, f) = open(&dir, 8);
    fill(&idx, 5);
    f.atomic_write.store(2, Ordering::SeqCst);
    idx.commit().expect("meta.json replace must be retried");
    assert_eq!(f.atomic_write.load(Ordering::SeqCst), 0);
    assert_eq!(idx.num_docs(), 5);
    // 재오픈 후에도 일관
    drop(idx);
    let idx =
        SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::all())).unwrap();
    assert_eq!(idx.num_docs(), 5);
}

#[test]
fn t_fts_23e_zero_attempts_policy_still_tries_once() {
    // Boundary: max_attempts 0/1 은 재시도 없이 한 번은 시도한다
    for attempts in [0, 1] {
        let dir = tempdir().unwrap();
        let (idx, f) = open(&dir, attempts);
        fill(&idx, 3);
        f.open_write.store(1, Ordering::SeqCst);
        assert!(idx.commit().is_err(), "attempts={attempts}");
        assert_eq!(f.open_write_calls.load(Ordering::SeqCst), 1);
    }
}
