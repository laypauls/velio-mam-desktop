//! DbPool 통합 테스트 (T-store-05 ~ T-store-07, P2-1).
//!
//! 분류: Happy (05 순서·읽기 풀) / Business rule (05b 우선순위) / Concurrency (06 WAL) / External (07 스레드 종료·패닉).

mod common;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use common::migrated_db;
use mam_db::{DbError, DbPool, PoolError, Priority};

const WAIT: Duration = Duration::from_secs(10);

fn pool_with_table(readers: usize) -> (tempfile::TempDir, DbPool) {
    let db = migrated_db();
    db.conn
        .execute_batch("CREATE TABLE t (n INTEGER NOT NULL, tag TEXT NOT NULL DEFAULT '')")
        .unwrap();
    let (dir, path) = (db.dir, db.db_path);
    drop(db.conn);
    let pool = DbPool::open(&path, readers).unwrap();
    (dir, pool)
}

fn count(pool: &DbPool) -> i64 {
    pool.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))?))
        .unwrap()
}

#[test]
fn t_store_05_writes_run_in_order_and_readers_query_concurrently() {
    // Arrange
    let (_dir, pool) = pool_with_table(4);
    let pool = Arc::new(pool);

    // Act: 한 스레드의 쓰기는 호출 순서대로 적용된다
    for n in 0..50 {
        pool.write(Priority::Low, move |c| {
            c.execute("INSERT INTO t (n) VALUES (?1)", [n])?;
            Ok(())
        })
        .unwrap();
    }
    // 읽기 8개 스레드가 동시에 같은 결과를 본다 (풀 4개를 빌려 쓰고 돌려준다)
    let readers: Vec<_> = (0..8)
        .map(|_| {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                pool.read(|c| {
                    let order: Vec<i64> = c
                        .prepare("SELECT n FROM t ORDER BY rowid")?
                        .query_map([], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    Ok(order)
                })
                .unwrap()
            })
        })
        .collect();

    // Assert
    let expected: Vec<i64> = (0..50).collect();
    for r in readers {
        assert_eq!(r.join().unwrap(), expected);
    }
}

#[test]
fn t_store_05b_read_connections_cannot_write() {
    let (_dir, pool) = pool_with_table(1);
    let result = pool.read(|c| Ok(c.execute("INSERT INTO t (n) VALUES (1)", [])?));
    assert!(matches!(result, Err(PoolError::Db(DbError::Sqlite(_)))));
    assert_eq!(count(&pool), 0);
}

#[test]
fn t_store_05c_high_priority_write_jumps_ahead_of_queued_low_priority_ones() {
    // Arrange: 쓰기 스레드를 게이트 작업으로 막고 Low -> High 순으로 대기열에 넣는다
    let (_dir, pool) = pool_with_table(1);
    let pool = Arc::new(pool);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let gate = {
        let pool = Arc::clone(&pool);
        std::thread::spawn(move || {
            pool.write(Priority::Low, move |_| {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(WAIT).unwrap();
                Ok(())
            })
        })
    };
    started_rx.recv_timeout(WAIT).unwrap();
    let submit = |p: Priority, tag: &'static str| {
        let pool = Arc::clone(&pool);
        std::thread::spawn(move || {
            pool.write(p, move |c| {
                c.execute("INSERT INTO t (n, tag) VALUES (0, ?1)", [tag])?;
                Ok(())
            })
        })
    };
    let low = submit(Priority::Low, "low");
    while pool.pending_writes() < 1 {
        std::thread::yield_now();
    }
    let high = submit(Priority::High, "high");
    while pool.pending_writes() < 2 {
        std::thread::yield_now();
    }

    // Act
    release_tx.send(()).unwrap();
    for h in [gate, low, high] {
        h.join().unwrap().unwrap();
    }

    // Assert
    let tags: Vec<String> = pool
        .read(|c| {
            Ok(c.prepare("SELECT tag FROM t ORDER BY rowid")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(tags, ["high", "low"]);
}

#[test]
fn t_store_06_readers_are_not_blocked_by_an_open_write_transaction() {
    // Arrange
    let (_dir, pool) = pool_with_table(2);
    let pool = Arc::new(pool);
    let (inserted_tx, inserted_rx) = mpsc::channel();
    let (commit_tx, commit_rx) = mpsc::channel::<()>();
    let writer = {
        let pool = Arc::clone(&pool);
        std::thread::spawn(move || {
            pool.write(Priority::High, move |c| {
                let tx = c.transaction()?;
                tx.execute("INSERT INTO t (n) VALUES (1)", [])?;
                inserted_tx.send(()).unwrap(); // 커밋 전
                commit_rx.recv_timeout(WAIT).unwrap();
                tx.commit()?;
                Ok(())
            })
        })
    };
    inserted_rx.recv_timeout(WAIT).unwrap();

    // Act: 쓰기 트랜잭션이 열린 채로 읽는다 (busy_timeout 5초 안에 즉시 반환해야 한다)
    let started = std::time::Instant::now();
    let during = count(&pool);
    let read_elapsed = started.elapsed();
    commit_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    let after = count(&pool);

    // Assert
    assert_eq!(during, 0, "uncommitted write must be invisible");
    assert!(read_elapsed < Duration::from_secs(2), "{read_elapsed:?}");
    assert_eq!(after, 1);
}

#[test]
fn t_store_07_panicking_write_reports_error_and_writer_keeps_working() {
    // Arrange
    let (_dir, pool) = pool_with_table(1);

    // Act: 트랜잭션 도중 패닉
    let panicked = pool.write(Priority::Low, |c| -> Result<(), DbError> {
        let tx = c.transaction()?;
        tx.execute("INSERT INTO t (n) VALUES (1)", [])?;
        panic!("boom");
    });
    let next = pool.write(Priority::Low, |c| {
        c.execute("INSERT INTO t (n) VALUES (2)", [])?;
        Ok(())
    });

    // Assert: 패닉한 쓰기는 롤백되고 다음 쓰기는 정상
    assert!(matches!(panicked, Err(PoolError::Panicked)));
    next.unwrap();
    let rows: Vec<i64> = pool
        .read(|c| {
            Ok(c.prepare("SELECT n FROM t")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?)
        })
        .unwrap();
    assert_eq!(rows, [2]);
}

#[test]
fn t_store_07b_writes_after_shutdown_fail_fast_and_queued_work_finishes() {
    // Arrange
    let (_dir, pool) = pool_with_table(1);
    pool.write(Priority::Low, |c| {
        c.execute("INSERT INTO t (n) VALUES (1)", [])?;
        Ok(())
    })
    .unwrap();

    // Act
    pool.shutdown();
    pool.shutdown(); // 멱등
    let late = pool.write(Priority::High, |_| Ok(()));

    // Assert: 멈추지 않고 오류, 읽기는 계속 가능
    assert!(matches!(late, Err(PoolError::WriterGone)));
    assert_eq!(count(&pool), 1);
}
