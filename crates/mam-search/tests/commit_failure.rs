//! commit 실패 복구 테스트 (`Directory` 래퍼로 I/O 오류를 결정적으로 주입).
//! 분류: T-fts-20 External failure / Business rule(손실 없음) / T-fts-21 External failure(poison)

mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use mam_search::FilenameStrategy::A;
use mam_search::{IndexOptions, RetryDirectory, SearchError, SearchIndex, UpsertOutcome};
use tantivy::directory::FileSlice;
use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    Directory, DirectoryLock, FileHandle, Lock, WatchCallback, WatchHandle, WritePtr,
};
use tempfile::tempdir;

/// 세그먼트 데이터 파일(.idx/.store/.fast) `open_write` 를 `fail` 번 실패시킨다 (os error 5 모사).
/// 재시도 계층 **위**에 놓여 "재시도로도 복구되지 않는 지속 실패" 를 결정적으로 만든다.
#[derive(Clone, Debug)]
struct FlakyDir {
    inner: RetryDirectory,
    fail: Arc<AtomicUsize>,
    /// 0 이 아니면 `meta.json` 읽기를 실패시킨다 (rollback 의 새 writer 생성 실패 모사).
    fail_meta_read: Arc<AtomicUsize>,
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
        let data_file = matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("idx" | "store" | "fast")
        );
        if data_file
            && self
                .fail
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            let e = io::Error::from(io::ErrorKind::PermissionDenied);
            return Err(OpenWriteError::wrap_io_error(e, PathBuf::from(p)));
        }
        self.inner.open_write(p)
    }
    fn atomic_read(&self, p: &Path) -> Result<Vec<u8>, OpenReadError> {
        if self.fail_meta_read.load(Ordering::SeqCst) != 0 && p.ends_with("meta.json") {
            let e = io::Error::from(io::ErrorKind::PermissionDenied);
            return Err(OpenReadError::wrap_io_error(e, PathBuf::from(p)));
        }
        self.inner.atomic_read(p)
    }
    fn atomic_write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
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

fn open_flaky(dir: &tempfile::TempDir) -> (SearchIndex, Arc<AtomicUsize>) {
    let (idx, fail, _) = open_flaky2(dir);
    (idx, fail)
}

fn open_flaky2(dir: &tempfile::TempDir) -> (SearchIndex, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let fail = Arc::new(AtomicUsize::new(0));
    let meta = Arc::new(AtomicUsize::new(0));
    let (f, m) = (fail.clone(), meta.clone());
    let idx = SearchIndex::open_or_create_with_directory(
        &dir.path().join("idx"),
        small(IndexOptions::all()),
        move |inner| {
            Box::new(FlakyDir {
                inner,
                fail: f,
                fail_meta_read: m,
            })
        },
    )
    .unwrap();
    (idx, fail, meta)
}

fn batch(range: std::ops::Range<u64>, rev: u64, prefix: &str) -> Vec<mam_search::AssetDoc> {
    range
        .map(|i| {
            let mut d = doc(i, &format!("{prefix}{i}.jpg"));
            d.revision = rev;
            d
        })
        .collect()
}

#[test]
fn t_fts_20_failed_commit_discards_batch_and_reapply_loses_nothing() {
    let dir = tempdir().unwrap();
    let (idx, fail) = open_flaky(&dir);
    for d in batch(0..100, 1, "base") {
        idx.upsert(&d).unwrap();
    }
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 100);

    // 30건 갱신 배치(rev 2) 를 쌓고, 데이터 파일 쓰기가 실패하도록 한 뒤 commit
    let updates = batch(0..30, 2, "upd");
    for d in &updates {
        assert_eq!(idx.upsert(d).unwrap(), UpsertOutcome::Applied);
    }
    fail.store(1, Ordering::SeqCst);
    let err = idx.commit().expect_err("commit must fail");
    assert!(matches!(err, SearchError::Tantivy(_)), "{err:?}");
    assert_eq!(fail.load(Ordering::SeqCst), 0, "failure was injected");

    // 배치는 버려졌다: 이전 commit 상태 그대로 (옛 문서 100건, 새 이름은 없음)
    assert_eq!(idx.num_docs(), 100);
    assert_eq!(ids(&idx, &req("base5", A)), [5]);
    assert!(ids(&idx, &req("upd5", A)).is_empty());

    // 계약대로 배치를 다시 적용하면 revision 가드에 막히지 않고(대기 맵이 비워짐) 모두 반영된다
    for d in &updates {
        assert_eq!(idx.upsert(d).unwrap(), UpsertOutcome::Applied);
    }
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 100, "no doc loss and no duplicates");
    for i in 0..30 {
        assert_eq!(ids(&idx, &req(&format!("upd{i}"), A)), [i], "upd{i}");
        assert!(ids(&idx, &req(&format!("base{i}"), A)).is_empty());
    }
    assert_eq!(ids(&idx, &req("base50", A)), [50]);

    // 이후 정상 동작 + 재오픈 후에도 동일
    drop(idx);
    let idx =
        SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::all())).unwrap();
    assert_eq!(idx.num_docs(), 100);
}

#[test]
fn t_fts_20b_repeated_failures_then_success() {
    let dir = tempdir().unwrap();
    let (idx, fail) = open_flaky(&dir);
    for round in 0..3 {
        for d in batch(0..20, 1, "n") {
            idx.upsert(&d).unwrap();
        }
        fail.store(1, Ordering::SeqCst);
        assert!(idx.commit().is_err(), "round {round}");
        assert_eq!(idx.num_docs(), 0);
    }
    for d in batch(0..20, 1, "n") {
        idx.upsert(&d).unwrap();
    }
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 20);
    assert_eq!(ids(&idx, &req("n7", A)), [7]);
}

#[test]
fn t_fts_21_failed_rollback_poisons_writer_until_reopen() {
    let dir = tempdir().unwrap();
    let (idx, fail, meta_fail) = open_flaky2(&dir);
    idx.upsert(&doc(1, "a.jpg")).unwrap();
    idx.commit().unwrap();
    idx.upsert(&doc(2, "b.jpg")).unwrap();

    // commit 실패 + rollback(새 writer 생성) 실패
    fail.store(1, Ordering::SeqCst);
    meta_fail.store(1, Ordering::SeqCst);
    assert!(idx.commit().is_err());
    meta_fail.store(0, Ordering::SeqCst);

    // poisoned: 이후 모든 쓰기가 전용 오류, 패닉 없음 (재-rollback 시도 없음)
    assert!(matches!(
        idx.upsert(&doc(3, "c.jpg")),
        Err(SearchError::WriterPoisoned)
    ));
    assert!(matches!(
        idx.upsert_unguarded(&doc(3, "c.jpg")),
        Err(SearchError::WriterPoisoned)
    ));
    assert!(matches!(idx.delete(1), Err(SearchError::WriterPoisoned)));
    assert!(matches!(idx.commit(), Err(SearchError::WriterPoisoned)));
    // 읽기는 마지막 commit 상태로 계속 동작
    assert_eq!(ids(&idx, &req("a.jpg", A)), [1]);

    // 핸들을 버리고 다시 열면 복구 (디렉터리 잠금도 풀려 있어야 한다)
    drop(idx);
    let idx =
        SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::all())).unwrap();
    assert_eq!(idx.num_docs(), 1);
    idx.upsert(&doc(2, "b.jpg")).unwrap();
    idx.commit().unwrap();
    assert_eq!(idx.num_docs(), 2);
}
