//! DB 연결 구조 — 쓰기 연결 1개(전용 스레드, 우선순위 2단계 큐) + 읽기 전용 연결 풀 (P2-1, D3).
//!
//! - 모든 쓰기는 쓰기 스레드 하나에서 직렬 실행한다 -> SQLITE_BUSY 경합 없음. 사용자 편집은 `High`,
//!   대량 스캔·AI 결과는 `Low` — 큐에서 `High` 가 항상 먼저 나간다.
//! - 읽기는 호출 스레드에서 실행하며 `query_only` 연결이라 실수로 쓸 수 없다. WAL 이라 쓰기와 서로 막지 않는다.
//! - 쓰기 클로저가 패닉해도 스레드는 살아남고 호출자는 `Panicked` 를 받는다. 쓰기 스레드가 없어졌거나
//!   종료되면 이후 명령은 즉시 `WriterGone` 이다(멈추지 않는다).
//!   (`panic = "abort"` 로 빌드한 릴리스에서는 패닉이 프로세스를 끝낸다 — 이 복구는 개발·테스트 빌드용 방어선이다.)

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::Connection;

mod deadline;

use crate::connection::open;
use crate::error::DbError;

/// 기본 읽기 연결 수.
pub const DEFAULT_READERS: usize = 4;
/// 읽기 연결을 기다리는 최대 시간.
pub const READ_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// 사용자 편집(태그·메모 등)
    High,
    /// 대량 스캔·AI 결과 반영
    Low,
}

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// 쓰기 스레드가 종료됐다(또는 종료 중이다).
    #[error("database writer is not running")]
    WriterGone,
    /// 쓰기 클로저가 패닉했다 — 트랜잭션은 롤백됐고 쓰기 스레드는 계속 동작한다.
    #[error("database write task panicked")]
    Panicked,
    #[error("timed out waiting for a database read connection")]
    ReadTimeout,
    /// `read_until`·`write_until` 의 마감에 닿았다 — 쓰기는 취소돼 이후에도 실행되지 않는다.
    #[error("database call deadline exceeded")]
    DeadlineExceeded,
}

impl From<rusqlite::Error> for PoolError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.into())
    }
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

#[derive(Default)]
struct Queue {
    high: VecDeque<Job>,
    low: VecDeque<Job>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct DbPool {
    shared: Arc<Shared>,
    writer: Mutex<Option<JoinHandle<()>>>,
    readers: Mutex<Vec<Connection>>,
    reader_free: Condvar,
}

impl DbPool {
    /// 쓰기 연결 1개 + 읽기 연결 `readers` 개를 연다. 마이그레이션은 풀을 열기 **전에** 끝나 있어야 한다.
    pub fn open(path: &Path, readers: usize) -> Result<Self, DbError> {
        let write_conn = open(path)?;
        let mut read_conns = Vec::with_capacity(readers.max(1));
        for _ in 0..readers.max(1) {
            let conn = open(path)?;
            conn.pragma_update(None, "query_only", true)?;
            read_conns.push(conn);
        }
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("mam-db-writer".into())
            .spawn(move || writer_loop(&worker, write_conn))
            .map_err(DbError::Io)?;
        Ok(Self {
            shared,
            writer: Mutex::new(Some(handle)),
            readers: Mutex::new(read_conns),
            reader_free: Condvar::new(),
        })
    }

    /// 쓰기 스레드에서 `f` 를 실행하고 결과를 기다린다(블로킹 — async command 에서는 `spawn_blocking` 안에서 호출).
    /// `f` 안에서 트랜잭션을 열고 커밋까지 끝내야 한다. 자산 변경과 `search_outbox` 기록은 같은 트랜잭션이어야 한다.
    pub fn write<T, F>(&self, priority: Priority, f: F) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let outcome = match catch_unwind(AssertUnwindSafe(|| f(conn))) {
                Ok(r) => r.map_err(PoolError::Db),
                Err(_) => Err(PoolError::Panicked),
            };
            let _ = tx.send(outcome);
        });
        self.enqueue(priority, job)?;
        // 작업이 실행되기 전에 스레드가 사라지면 job(과 tx)이 버려져 recv 가 즉시 실패한다
        rx.recv().map_err(|_| PoolError::WriterGone)?
    }

    fn enqueue(&self, priority: Priority, job: Job) -> Result<(), PoolError> {
        {
            let mut q = lock(&self.shared.queue);
            if q.closed {
                return Err(PoolError::WriterGone);
            }
            match priority {
                Priority::High => q.high.push_back(job),
                Priority::Low => q.low.push_back(job),
            }
        }
        self.shared.wake.notify_one();
        Ok(())
    }

    /// 읽기 전용 연결을 하나 빌려 `f` 를 호출 스레드에서 실행한다.
    pub fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, PoolError> {
        let conn = {
            let mut pool = lock(&self.readers);
            loop {
                if let Some(c) = pool.pop() {
                    break c;
                }
                let (guard, timeout) = self
                    .reader_free
                    .wait_timeout(pool, READ_WAIT)
                    .unwrap_or_else(|p| p.into_inner());
                pool = guard;
                if timeout.timed_out() && pool.is_empty() {
                    return Err(PoolError::ReadTimeout);
                }
            }
        };
        let guard = ReaderGuard {
            pool: self,
            conn: Some(conn),
        };
        let result = match guard.conn.as_ref() {
            Some(conn) => f(conn),
            None => return Err(PoolError::ReadTimeout), // 도달 불가: 가드는 Drop 전까지 연결을 가진다
        };
        drop(guard);
        result.map_err(PoolError::Db)
    }

    /// 쓰기 큐에서 아직 시작하지 않은 작업 수 (진단·시험용).
    pub fn pending_writes(&self) -> usize {
        let q = lock(&self.shared.queue);
        q.high.len() + q.low.len()
    }

    /// 새 쓰기를 거부하고, 이미 큐에 들어온 작업까지 마친 뒤 쓰기 스레드를 종료한다. 여러 번 호출해도 안전하다.
    pub fn shutdown(&self) {
        lock(&self.shared.queue).closed = true;
        self.shared.wake.notify_all();
        if let Some(handle) = lock(&self.writer).take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DbPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct ReaderGuard<'a> {
    pool: &'a DbPool,
    conn: Option<Connection>,
}

impl Drop for ReaderGuard<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            lock(&self.pool.readers).push(conn);
            self.pool.reader_free.notify_one();
        }
    }
}

/// 쓰기 스레드가 어떤 이유로든 끝나면 큐를 닫고 남은 작업을 버린다 — 기다리던 호출자가 `WriterGone` 을 받는다.
struct ExitGuard<'a>(&'a Shared);

impl Drop for ExitGuard<'_> {
    fn drop(&mut self) {
        let mut q = lock(&self.0.queue);
        q.closed = true;
        q.high.clear();
        q.low.clear();
    }
}

fn writer_loop(shared: &Shared, mut conn: Connection) {
    let _exit = ExitGuard(shared);
    loop {
        let job = {
            let mut q = lock(&shared.queue);
            loop {
                if let Some(job) = q.high.pop_front().or_else(|| q.low.pop_front()) {
                    break Some(job);
                }
                if q.closed {
                    break None;
                }
                q = shared.wake.wait(q).unwrap_or_else(|p| p.into_inner());
            }
        };
        match job {
            Some(job) => job(&mut conn),
            None => return,
        }
    }
}
