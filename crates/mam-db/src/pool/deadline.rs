//! 마감 인지 풀 호출 — 감독자 락을 쥔 호스트 호출(플러그인 게이트웨이)이 무기한 대기하지 않게 한다.
//!
//! - `read_until`: 읽기 연결을 `deadline` 까지만 기다린다.
//! - `write_until`: 쓰기 작업에 상태(Queued/Running/Cancelled)를 두고, 마감에 호출자가 Queued -> Cancelled 로 CAS 한다.
//!   CAS 에 이기면 쓰기 스레드가 그 작업을 건너뛴다 — **마감이 지난 쓰기는 나중에 실행되지 않는다**.
//!   지면(이미 Running)이면 짧은 단일 트랜잭션이 끝나길 기다려 결과를 정확히 돌려준다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use rusqlite::Connection;

use super::{DbPool, Job, PoolError, Priority, ReaderGuard, lock};
use crate::error::DbError;

const QUEUED: u8 = 0;
const RUNNING: u8 = 1;
const CANCELLED: u8 = 2;

impl DbPool {
    /// [`DbPool::read`] 와 같지만 읽기 연결을 `deadline` 까지만 기다린다. 이미 지났으면 연결을 건드리지 않고 `DeadlineExceeded`.
    pub fn read_until<T>(
        &self,
        deadline: Instant,
        f: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, PoolError> {
        if Instant::now() >= deadline {
            return Err(PoolError::DeadlineExceeded);
        }
        let guard = self
            .acquire_reader(deadline)
            .ok_or(PoolError::DeadlineExceeded)?;
        let result = match guard.conn.as_ref() {
            Some(conn) => f(conn),
            None => return Err(PoolError::DeadlineExceeded), // 도달 불가: 가드는 Drop 전까지 연결을 가진다
        };
        drop(guard);
        result.map_err(PoolError::Db)
    }

    /// [`DbPool::write`] 와 같지만 `deadline` 까지 시작하지 못한 작업은 취소한다(`DeadlineExceeded`, 이후에도 실행되지 않음).
    /// 이미 실행 중이면 끝날 때까지 기다려 실제 결과를 돌려준다. 마감이 이미 지났으면 큐에 넣지 않는다.
    pub fn write_until<T, F>(
        &self,
        priority: Priority,
        deadline: Instant,
        f: F,
    ) -> Result<T, PoolError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, DbError> + Send + 'static,
    {
        if Instant::now() >= deadline {
            return Err(PoolError::DeadlineExceeded);
        }
        let state = Arc::new(AtomicU8::new(QUEUED));
        let (tx, rx) = mpsc::sync_channel(1);
        let job_state = Arc::clone(&state);
        let job: Job = Box::new(move |conn| {
            if job_state
                .compare_exchange(QUEUED, RUNNING, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return; // 호출자가 이미 취소했다 — 실행하지 않는다
            }
            let outcome = match catch_unwind(AssertUnwindSafe(|| f(conn))) {
                Ok(r) => r.map_err(PoolError::Db),
                Err(_) => Err(PoolError::Panicked),
            };
            let _ = tx.send(outcome);
        });
        self.enqueue(priority, job)?;
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(PoolError::WriterGone),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if state
                    .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    Err(PoolError::DeadlineExceeded)
                } else {
                    rx.recv().map_err(|_| PoolError::WriterGone)?
                }
            }
        }
    }

    fn acquire_reader(&self, deadline: Instant) -> Option<ReaderGuard<'_>> {
        let mut pool = lock(&self.readers);
        let conn = loop {
            if let Some(c) = pool.pop() {
                break c;
            }
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                return None;
            }
            pool = self
                .reader_free
                .wait_timeout(pool, remain)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        };
        drop(pool);
        Some(ReaderGuard {
            pool: self,
            conn: Some(conn),
        })
    }
}
