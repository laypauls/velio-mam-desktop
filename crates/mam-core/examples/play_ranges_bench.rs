//! 재생 구간 측정 (설계 §3 성능 예산) — 합성 자산 N 개(기본 100만) + 컬렉션 10만 + 구간 보유 자산 1만(평균 3구간)에서
//! `play_ranges_get` / `play_ranges_set` / `collection_play_range_summary` 지연과 기존 `collection_play_queue` 를 잰다.
//!
//! 실행: cargo run --release -p mam-core --example play_ranges_bench -- --dir target/bench-play-ranges
//! 옵션: `--assets 1000000`, `--iters 300`. 끝나면 `--dir` 을 지운다 (합성 데이터가 디스크를 채운다).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::userdata::{PlayRange, UserData};
use mam_db::{DbPool, SystemClock, migrate, open};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == &format!("--{name}"))
        .and_then(|i| a.get(i + 1).cloned())
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn row(name: &str, budget: &str, mut v: Vec<f64>) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "| {name} | {} | {:.3} / {:.3} / {:.3} | {budget} |",
        v.len(),
        pct(&v, 50.0),
        pct(&v, 95.0),
        pct(&v, 99.0)
    );
}

fn timed<T>(iters: usize, mut f: impl FnMut(usize) -> T) -> Vec<f64> {
    (0..iters)
        .map(|i| {
            let t = Instant::now();
            std::hint::black_box(f(i));
            ms(t.elapsed())
        })
        .collect()
}

fn main() {
    let dir = PathBuf::from(arg("dir").unwrap_or_else(|| "target/bench-play-ranges".into()));
    let n: i64 = arg("assets")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = arg("iters").and_then(|s| s.parse().ok()).unwrap_or(300);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("mam.sqlite");

    // 생성: 모두 동영상(fps 25, 길이 1000 s). 컬렉션 1 = id%10==3 (10만), 구간 = id%100==3 인 1만 자산 × 평균 3구간
    let t = Instant::now();
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.join("backups"), &SystemClock).unwrap();
    conn.pragma_update(None, "synchronous", "OFF").unwrap();
    conn.execute(
        "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at)
         VALUES (1, 'fixed', 'v1', 'Main', 'C:/Media', 'online', 0)",
        [],
    )
    .unwrap();
    let tx = conn.transaction().unwrap();
    let seq = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < ?1)";
    tx.execute(
        &format!(
            "{seq} INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             SELECT i, 'video', 1000 + i % 5000, i, 0, 0 FROM n"
        ),
        [n],
    )
    .unwrap();
    tx.execute(
        &format!(
            "{seq} INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
             SELECT i, 1, 'd' || (i / 100) || '/f' || i || '.mp4', 'd' || (i / 100) || '/f' || i || '.mp4', 0 FROM n"
        ),
        [n],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO media_metadata (asset_id, duration_ms, fps) SELECT asset_id, 1000000, 25.0 FROM media_assets",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (1, 'c1', 'c1', 0)",
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO collection_assets (collection_id, asset_id, added_at, position)
         SELECT 1, asset_id, asset_id, row_number() OVER (ORDER BY (asset_id * 2654435761) % 1000003) * 1048576
         FROM media_assets WHERE asset_id % 10 = 3",
        [],
    )
    .unwrap();
    // 구간: id%100==3 -> 1만 자산, 구간 수 = 1 + (id/100)%5 (평균 3)
    tx.execute(
        "INSERT INTO asset_play_ranges (asset_id, start_ms, end_ms)
         SELECT a.asset_id, k.v * 10000, k.v * 10000 + 2000
         FROM media_assets a JOIN (SELECT 0 AS v UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4) k
           ON k.v <= (a.asset_id / 100) % 5
         WHERE a.asset_id % 100 = 3",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let ranged: i64 = conn
        .query_row(
            "SELECT count(DISTINCT asset_id) FROM asset_play_ranges",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM asset_play_ranges", [], |r| r.get(0))
        .unwrap();
    println!(
        "generated in {:.1}s: assets={n}, collection=100000, ranged assets={ranged}, range rows={rows}",
        t.elapsed().as_secs_f64()
    );
    drop(conn);

    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(SystemClock);
    let ud = UserData::new(Arc::clone(&pool), clock);

    // 1000구간 자산 하나 (비중첩: 500 ms 구간 + 400 ms 간격, 총 900 s < 길이 1000 s)
    let big_asset = 2;
    let big: Vec<PlayRange> = (0..1000)
        .map(|i| PlayRange {
            start_ms: i * 900,
            end_ms: i * 900 + 500,
        })
        .collect();
    ud.play_ranges_set(big_asset, &big).unwrap();

    println!("\n| 항목 | 반복 | P50 / P95 / P99 (ms) | 예산 |\n|---|---|---|---|");
    // get: 구간 보유 자산(평균 3구간) 무작위, 1000구간 자산
    let ids: Vec<i64> = (0..iters as i64)
        .map(|i| 3 + 100 * ((i * 7919) % 10_000))
        .collect();
    row(
        "play_ranges_get (평균 3구간)",
        "P95 < 5",
        timed(iters, |i| ud.play_ranges_get(ids[i]).unwrap()),
    );
    row(
        "play_ranges_get (1000구간)",
        "P95 < 5",
        timed(iters, |_| ud.play_ranges_get(big_asset).unwrap()),
    );
    // set: 3구간 교체, 1000구간 교체 (자산 3 은 구간 보유 자산이라 DELETE+INSERT 가 실제로 일어난다)
    let small = [
        PlayRange {
            start_ms: 0,
            end_ms: 1000,
        },
        PlayRange {
            start_ms: 5000,
            end_ms: 6000,
        },
    ];
    row(
        "play_ranges_set (2구간)",
        "-",
        timed(iters, |i| ud.play_ranges_set(ids[i], &small).unwrap()),
    );
    row(
        "play_ranges_set (1000구간)",
        "P95 < 20",
        timed(iters, |_| ud.play_ranges_set(big_asset, &big).unwrap()),
    );
    // summary: 컬렉션 10만 안에 구간 보유 자산 1만
    let first = ud.collection_play_range_summary(1).unwrap();
    println!("(summary rows = {})", first.len());
    row(
        "collection_play_range_summary (100k 컬렉션, 구간 자산 ~1만)",
        "P95 < 50",
        timed(iters.min(100), |_| {
            ud.collection_play_range_summary(1).unwrap()
        }),
    );
    // 기존 대기열 (변경 없음 확인용 기준선)
    row(
        "collection_play_queue (100k) [기존]",
        "P95 ~469 (기존 기록)",
        timed(iters.min(30), |_| ud.collection_play_queue(1).unwrap()),
    );

    drop(ud);
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
    println!("\ncleaned {}", dir.display());
}
