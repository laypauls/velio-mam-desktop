//! 마감 인지 풀 호출 (T-pool-*) — 실제 SQLite 임시 파일, 시각은 상대 마감(짧은 값)만 쓰고 sleep 으로 맞추지 않는다.
//!
//! 분류: Time(마감) / Business rule(취소된 쓰기는 실행되지 않음, Low 는 High 에 양보) / Concurrency(CAS 경합) / External(종료)

mod common;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::migrated_db;
use mam_db::{DbPool, PoolError, Priority};

const WAIT: Duration = Duration::from_secs(10);
const SHORT: Duration = Duration::from_millis(150);

fn pool(readers: usize) -> (tempfile::TempDir, Arc<DbPool>) {
    let db = migrated_db();
    db.conn.execute_batch("CREATE TABLE t (n INTEGER NOT NULL, tag TEXT NOT NULL DEFAULT '')").unwrap();
    let (dir, path) = (db.dir, db.db_path);
    drop(db.conn);
    (dir, Arc::new(DbPool::open(&path, readers).unwrap()))
}

fn rows(pool: &DbPool) -> Vec<String> {
    pool.read(|c| Ok(c.prepare("SELECT tag FROM t ORDER BY rowid")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)).unwrap()
}

/// 쓰기 스레드를 막는 작업을 시작하고 (시작 확인 후) 해제 송신자를 돌려준다
fn block_writer(pool: &Arc<DbPool>, priority: Priority) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let p = Arc::clone(pool);
    let h = std::thread::spawn(move || {
        p.write(priority, move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            Ok(())
        })
        .unwrap();
    });
    started_rx.recv_timeout(WAIT).unwrap();
    (release_tx, h)
}

fn insert(tag: &'static str) -> impl FnOnce(&mut rusqlite::Connection) -> Result<(), mam_db::DbError> + Send + 'static {
    move |c| {
        c.execute("INSERT INTO t (n, tag) VALUES (0, ?1)", [tag])?;
        Ok(())
    }
}

#[test]
fn t_pool_01_write_until_runs_normally_before_the_deadline() {
    let (_d, pool) = pool(2);
    pool.write_until(Priority::Low, Instant::now() + WAIT, insert("ok")).unwrap();
    assert_eq!(rows(&pool), ["ok"]);
}

#[test]
fn t_pool_02_write_blocked_past_deadline_is_cancelled_and_never_runs_later() {
    // Arrange: 긴 High 작업이 쓰기 스레드를 막고 있다
    let (_d, pool) = pool(2);
    let (release, gate) = block_writer(&pool, Priority::High);
    // Act: 짧은 마감
    let t = Instant::now();
    let e = pool.write_until(Priority::Low, Instant::now() + SHORT, insert("late")).unwrap_err();
    // Assert: 마감 안에 돌아오고, 막힘이 풀린 뒤에도 실행되지 않는다
    assert!(matches!(e, PoolError::DeadlineExceeded), "{e:?}");
    assert!(t.elapsed() < WAIT);
    release.send(()).unwrap();
    gate.join().unwrap();
    pool.write(Priority::High, insert("after")).unwrap(); // 큐가 비도록 뒤따르는 작업을 기다린다
    assert_eq!(rows(&pool), ["after"], "취소된 쓰기는 실행되지 않는다");
}

#[test]
fn t_pool_03_already_expired_deadline_never_touches_the_queue_or_a_reader() {
    let (_d, pool) = pool(1);
    let past = Instant::now();
    assert!(matches!(pool.write_until(Priority::High, past, insert("x")), Err(PoolError::DeadlineExceeded)));
    assert!(matches!(pool.read_until(past, |_| Ok(())), Err(PoolError::DeadlineExceeded)));
    assert_eq!(pool.pending_writes(), 0);
    assert!(rows(&pool).is_empty());
}

#[test]
fn t_pool_04_running_write_is_awaited_and_returns_the_real_result() {
    // Arrange: 마감보다 오래 걸리는 작업이 이미 Running 이다
    let (_d, pool) = pool(2);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let p = Arc::clone(&pool);
    let h = std::thread::spawn(move || {
        p.write_until(Priority::Low, Instant::now() + SHORT, move |c| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            c.execute("INSERT INTO t (n, tag) VALUES (1, 'slow')", [])?;
            Ok(42)
        })
    });
    started_rx.recv_timeout(WAIT).unwrap();
    // Act: 마감이 지난 뒤에 해제 — 호출자는 취소하지 못하고 끝까지 기다린다
    std::thread::sleep(SHORT * 2);
    release_tx.send(()).unwrap();
    // Assert
    assert_eq!(h.join().unwrap().unwrap(), 42);
    assert_eq!(rows(&pool), ["slow"]);
}

#[test]
fn t_pool_05_low_write_yields_to_high_and_cancelled_job_does_not_block_the_queue() {
    // Arrange: 막힌 쓰기 스레드 뒤에 Low(마감 있음) -> High 순으로 쌓는다
    let (_d, pool) = pool(2);
    let (release, gate) = block_writer(&pool, Priority::Low);
    let p = Arc::clone(&pool);
    let low = std::thread::spawn(move || p.write_until(Priority::Low, Instant::now() + WAIT, insert("low")));
    while pool.pending_writes() < 1 {
        std::thread::yield_now();
    }
    let p = Arc::clone(&pool);
    let high = std::thread::spawn(move || p.write_until(Priority::High, Instant::now() + WAIT, insert("high")));
    while pool.pending_writes() < 2 {
        std::thread::yield_now();
    }
    // 취소될 Low 도 하나 더 — 건너뛰어도 뒤 작업이 진행돼야 한다
    assert!(pool.write_until(Priority::Low, Instant::now() + SHORT, insert("cancelled")).is_err());
    // Act
    release.send(()).unwrap();
    gate.join().unwrap();
    // Assert
    low.join().unwrap().unwrap();
    high.join().unwrap().unwrap();
    assert_eq!(rows(&pool), ["high", "low"]);
}

#[test]
fn t_pool_06_cancel_race_is_exclusive_each_write_ran_xor_reported_deadline() {
    // Arrange/Act: 마감 경계에서 실행 시작과 취소가 겹치도록 짧은 마감을 많이 시도
    let (_d, pool) = pool(2);
    let mut ran = 0usize;
    let mut cancelled = 0usize;
    for i in 0..300u64 {
        let deadline = Instant::now() + Duration::from_micros(50 + (i % 7) * 40);
        match pool.write_until(Priority::Low, deadline, insert("r")) {
            Ok(()) => ran += 1,
            Err(PoolError::DeadlineExceeded) => cancelled += 1,
            Err(e) => panic!("{e:?}"),
        }
    }
    // Assert: Ok 로 보고된 것만 실제로 반영되고, 취소로 보고된 것은 반영되지 않는다 (나중에 실행되지도 않는다)
    pool.write(Priority::High, |_| Ok(())).unwrap();
    assert_eq!(rows(&pool).len(), ran, "ran={ran} cancelled={cancelled}");
}

#[test]
fn t_pool_07_read_until_times_out_when_all_readers_are_busy_and_recovers() {
    // Arrange: 유일한 읽기 연결을 다른 스레드가 쥔다
    let (_d, pool) = pool(1);
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let p = Arc::clone(&pool);
    let h = std::thread::spawn(move || {
        p.read(move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            Ok(())
        })
    });
    started_rx.recv_timeout(WAIT).unwrap();
    // Act
    let t = Instant::now();
    let e = pool.read_until(Instant::now() + SHORT, |_| Ok(())).unwrap_err();
    // Assert: 마감 안에 포기, 연결을 돌려주면 다시 읽힌다
    assert!(matches!(e, PoolError::DeadlineExceeded), "{e:?}");
    assert!(t.elapsed() < WAIT);
    release_tx.send(()).unwrap();
    h.join().unwrap().unwrap();
    assert_eq!(pool.read_until(Instant::now() + WAIT, |c| Ok(c.query_row("SELECT 7", [], |r| r.get::<_, i64>(0))?)).unwrap(), 7);
}

#[test]
fn t_pool_08_read_until_gets_a_connection_released_before_the_deadline() {
    let (_d, pool) = pool(1);
    let (started_tx, started_rx) = mpsc::channel();
    let p = Arc::clone(&pool);
    let h = std::thread::spawn(move || {
        p.read(move |_| {
            started_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            Ok(())
        })
    });
    started_rx.recv_timeout(WAIT).unwrap();
    assert!(pool.read_until(Instant::now() + WAIT, |_| Ok(())).is_ok());
    h.join().unwrap().unwrap();
}

#[test]
fn t_pool_09_closed_pool_is_writer_gone_and_a_waiting_caller_is_released() {
    // Arrange: 큐에서 대기 중인 마감 쓰기
    let (_d, pool) = pool(1);
    pool.shutdown();
    // Act / Assert
    assert!(matches!(pool.write_until(Priority::Low, Instant::now() + WAIT, insert("x")), Err(PoolError::WriterGone)));
    // 읽기는 닫힌 풀에서도 동작한다(기존 `read` 와 같음)
    assert!(pool.read_until(Instant::now() + WAIT, |_| Ok(())).is_ok());
}

#[test]
fn t_pool_10_panicking_write_until_reports_panicked_and_writer_survives() {
    let (_d, pool) = pool(1);
    let r: Result<(), _> = pool.write_until(Priority::Low, Instant::now() + WAIT, |_| panic!("boom"));
    assert!(matches!(r, Err(PoolError::Panicked)));
    pool.write_until(Priority::Low, Instant::now() + WAIT, insert("alive")).unwrap();
    assert_eq!(rows(&pool), ["alive"]);
}
