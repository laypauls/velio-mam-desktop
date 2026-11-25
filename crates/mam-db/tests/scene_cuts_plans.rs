//! 장면 컷 질의 계획 계약과 규모 측정 (T-scene-13).
//!
//! 분류: Performance — EXPLAIN QUERY PLAN 으로 인덱스 사용·TEMP B-TREE 없음을 고정한다 (앱이 쓰는 SQL 상수를 그대로 검사).
//! 규모 측정은 `#[ignore]` 벤치:
//! `cargo test -p mam-db --release --test scene_cuts_plans -- --ignored --nocapture`
//! (환경변수 `SCENE_BENCH_ASSETS` 로 자산 수 변경, 기본 1,000,000 중 30% 영상)

mod common;
mod scene_support;

use std::time::Instant;

use common::{TestDb, migrated_db};
use mam_db::scene_cuts::{
    self, BACKFILL_BATCH, BACKFILL_COUNT_SQL, BACKFILL_PAGE_SQL, CutSource, REQUEUE_SQL,
    add_user_tx, backfill_batch_tx, backfill_count, encode, remove_tx, requeue_pending_tx,
};
use rusqlite::Connection;
use scene_support::NOW;

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    // 파라미터 자리는 값으로 치환해 plan 만 본다
    let sql = sql.replace("?1", "5").replace("?2", "1000");
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const GET_SQL: &str = "SELECT state, sensitivity, detector_version, requested_sensitivity,
                auto_cuts, user_cuts, suppressed, error, updated_at
         FROM asset_scene_cuts WHERE asset_id = ?1";

fn assert_plan(conn: &Connection, sql: &str, expected: &[&str]) {
    let details = plan(conn, sql);
    let joined = details.join(" | ");
    for want in expected {
        assert!(
            details.iter().any(|d| d.contains(want)),
            "expected `{want}` in plan: {joined}"
        );
    }
    assert!(
        !details.iter().any(|d| d.contains("TEMP B-TREE")),
        "temp b-tree in plan: {joined}"
    );
}

#[test]
fn t_scene_13_get_is_a_rowid_point_lookup() {
    // Arrange / Act / Assert
    let db = migrated_db();
    assert_plan(
        &db.conn,
        GET_SQL,
        &["SEARCH asset_scene_cuts USING INTEGER PRIMARY KEY (rowid=?)"],
    );
}

#[test]
fn t_scene_13_backfill_page_and_count_walk_the_type_status_index_and_probe_by_primary_key() {
    // Arrange / Act / Assert
    let db = migrated_db();
    assert_plan(
        &db.conn,
        BACKFILL_PAGE_SQL,
        &[
            "SEARCH a USING COVERING INDEX idx_media_assets_type_status (media_type=? AND status=? AND rowid>?)",
            "SEARCH s USING INTEGER PRIMARY KEY (rowid=?)",
        ],
    );
    assert_plan(
        &db.conn,
        BACKFILL_COUNT_SQL,
        &[
            "SEARCH a USING COVERING INDEX idx_media_assets_type_status (media_type=? AND status=?)",
            "SEARCH s USING INTEGER PRIMARY KEY (rowid=?)",
        ],
    );
    for sql in [BACKFILL_PAGE_SQL, BACKFILL_COUNT_SQL] {
        assert!(
            !plan(&db.conn, sql).iter().any(|d| d.starts_with("SCAN")),
            "full scan: {:?}",
            plan(&db.conn, sql)
        );
    }
}

#[test]
fn t_scene_13_requeue_scans_only_the_partial_pending_index_and_probes_jobs_by_asset() {
    // Arrange / Act / Assert: 행 전체가 아니라 부분 인덱스(pending)만 훑고, 대기 작업은 (kind, asset_id) 부분 UNIQUE 로 찾는다
    let db = migrated_db();
    assert_plan(
        &db.conn,
        REQUEUE_SQL,
        &[
            "SCAN s USING COVERING INDEX idx_asset_scene_cuts_pending",
            "SEARCH j USING COVERING INDEX uq_jobs_pending_asset (kind=? AND asset_id=?)",
        ],
    );
    let scans: Vec<_> = plan(&db.conn, REQUEUE_SQL)
        .into_iter()
        .filter(|d| d.starts_with("SCAN"))
        .collect();
    assert_eq!(scans.len(), 1, "{scans:?}");
}

// --- 규모 측정 (수동 실행) --------------------------------------------------

fn pct(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let idx = ((samples.len() as f64 * p).ceil() as usize).clamp(1, samples.len()) - 1;
    samples[idx]
}

fn report(name: &str, samples: &mut [f64]) {
    println!(
        "BENCH {name}: n={} P50={:.3}ms P95={:.3}ms P99={:.3}ms max={:.3}ms",
        samples.len(),
        pct(samples, 0.50),
        pct(samples, 0.95),
        pct(samples, 0.99),
        pct(samples, 1.0)
    );
}

fn file_mb(db: &TestDb) -> f64 {
    let mut total = std::fs::metadata(&db.db_path).map(|m| m.len()).unwrap_or(0);
    let mut wal = db.db_path.clone().into_os_string();
    wal.push("-wal");
    total += std::fs::metadata(wal).map(|m| m.len()).unwrap_or(0);
    total as f64 / 1_048_576.0
}

#[test]
#[ignore = "규모 측정 — 수동 실행 (--release --ignored --nocapture)"]
fn t_scene_13_bench_at_scale() {
    // Arrange: N 자산(30% 영상, 3% 이미지 아님…) — 영상은 id % 10 < 3
    let total: i64 = std::env::var("SCENE_BENCH_ASSETS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let mut db = migrated_db();
    let t = Instant::now();
    db.conn
        .execute_batch(&format!(
            "BEGIN;
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {total})
             INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             SELECT i, CASE i % 10 WHEN 0 THEN 'video' WHEN 1 THEN 'video' WHEN 2 THEN 'video' WHEN 3 THEN 'audio' ELSE 'image' END, 1, 0, 0, 0 FROM n;
             COMMIT;"
        ))
        .unwrap();
    let videos: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM media_assets WHERE media_type = 'video'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    println!(
        "BENCH setup: {total} assets, {videos} videos in {:.1}s",
        t.elapsed().as_secs_f64()
    );

    // 영상의 1/3 은 평균 300컷 자동 추출 완료 행 (BLOB ~1.2KB)
    let cuts: Vec<i64> = (0..300).map(|k| 1000 + k * 7919).collect();
    let blob = encode(&cuts).unwrap();
    let with_rows: Vec<i64> = (1..=total).filter(|i| i % 10 < 3 && i % 3 == 0).collect();
    {
        let tx = db.conn.transaction().unwrap();
        let mut ins = tx
            .prepare("INSERT INTO asset_scene_cuts (asset_id, state, sensitivity, detector_version, auto_cuts, updated_at) VALUES (?1, 'done', 'medium', 1, ?2, 0)")
            .unwrap();
        for id in &with_rows {
            ins.execute(rusqlite::params![id, blob]).unwrap();
        }
        drop(ins);
        tx.commit().unwrap();
    }
    let size_before = file_mb(&db);
    // 2만 컷 행 (add/remove 상한 비용)
    let big: Vec<i64> = (0..20_000).map(|k| 1000 + k * 1000).collect();
    let big_id = with_rows[0];
    db.conn
        .execute(
            "UPDATE asset_scene_cuts SET auto_cuts = ?2 WHERE asset_id = ?1",
            rusqlite::params![big_id, encode(&big).unwrap()],
        )
        .unwrap();
    println!(
        "BENCH rows: {} scene rows, db {:.0} MB",
        with_rows.len(),
        size_before
    );

    // Act / Report: get (무작위 행 있는 자산)
    let mut s = Vec::new();
    for k in 0..3000usize {
        let id = with_rows[(k * 7919) % with_rows.len()];
        let t = Instant::now();
        let r = scene_cuts::get(&db.conn, id).unwrap().unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(!r.auto.is_empty());
    }
    report("get(300 cuts)", &mut s);
    let mut s = Vec::new();
    for _ in 0..200 {
        let t = Instant::now();
        let r = scene_cuts::get(&db.conn, big_id).unwrap().unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(r.auto.len(), 20_000);
    }
    report("get(20000 cuts)", &mut s);

    // add / remove on the 20k-cut row (트랜잭션·커밋 포함)
    let mut s = Vec::new();
    for k in 0..100i64 {
        let tx = db
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let t = Instant::now();
        add_user_tx(&tx, big_id, 500_000 + k * 5000 + 333, Some(25.0), None, NOW).unwrap();
        tx.commit().unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("add_user(20000 auto cuts)", &mut s);
    let mut s = Vec::new();
    for k in 0..100i64 {
        let tx = db
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let t = Instant::now();
        remove_tx(
            &tx,
            big_id,
            1000 + k * 1000,
            CutSource::Auto,
            Some(25.0),
            NOW,
        )
        .unwrap();
        tx.commit().unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("remove_auto(20000 cuts)", &mut s);

    // backfill count / pages
    let expected_targets = videos - with_rows.len() as i64;
    let mut s = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let n = backfill_count(&db.conn).unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(n, expected_targets);
    }
    report("backfill_count", &mut s);
    let mut s = Vec::new();
    let mut after = 0;
    let all = Instant::now();
    loop {
        let t = Instant::now();
        let tx = db.conn.unchecked_transaction().unwrap();
        let b = backfill_batch_tx(&tx, after, BACKFILL_BATCH, NOW).unwrap();
        tx.commit().unwrap();
        s.push(t.elapsed().as_secs_f64() * 1000.0);
        match b.last_id {
            Some(id) => after = id,
            None => break,
        }
    }
    println!(
        "BENCH backfill total: {expected_targets} videos in {:.1}s",
        all.elapsed().as_secs_f64()
    );
    report("backfill_batch(1000)", &mut s);
    assert_eq!(backfill_count(&db.conn).unwrap(), 0);

    // requeue: 작업이 모두 있는 상태 / 작업을 지운 상태
    let t = Instant::now();
    let n = requeue_pending_tx(&db.conn, NOW).unwrap();
    println!(
        "BENCH requeue(all have jobs): {n} in {:.1}ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    db.conn
        .execute("DELETE FROM jobs WHERE kind = 'scene'", [])
        .unwrap();
    let t = Instant::now();
    let tx = db.conn.unchecked_transaction().unwrap();
    let n = requeue_pending_tx(&tx, NOW).unwrap();
    tx.commit().unwrap();
    println!(
        "BENCH requeue(no jobs): {n} in {:.1}ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    println!(
        "BENCH db size: {:.0} MB -> {:.0} MB (+{:.0} MB for {expected_targets} pending rows + jobs)",
        size_before,
        file_mb(&db),
        file_mb(&db) - size_before
    );
}
