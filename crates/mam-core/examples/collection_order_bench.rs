//! 컬렉션 순서 정렬 측정 (설계 §3) — 합성 자산 N 개(기본 100만)에 컬렉션 10k·100k 를 만들어 페이지·seek·질의·재배치·
//! 소속 집계·재생 대기열 지연을 잰다. 실제 경로 그대로: 순서 맵 읽기(SQLite) + 조정기(Tantivy) + 하이드레이션(SQLite).
//!
//! 실행 (2단계 — 측정 프로세스를 새로 띄워 첫 요청 지연을 따로 본다):
//!   cargo run --release -p mam-core --example collection_order_bench -- --phase gen --dir target/bench-collection-order
//!   cargo run --release -p mam-core --example collection_order_bench -- --phase run --dir target/bench-collection-order
//! 옵션: `--assets 1000000`, `--iters 200`. 끝나면 `--dir` 을 지운다 (합성 데이터가 디스크를 채운다).
//!
//! 컬렉션: 1 = 10k (id % 100 == 0), 2 = 100k (id % 10 == 3), 3 = 100k 중 1% 가 위치 NULL (id % 10 == 7).
//! 위치는 id 와 무관한 순열을 `rank × 2^20` 로 채운 상태(= V12 직후)다.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::query::{QueryService, hydrate_page};
use mam_core::userdata::{ReorderTarget, UserData};
use mam_db::{DbPool, SystemClock, migrate, open};
use mam_search::coordinator::{
    CollectionOrder, CoordRequest, Direction, SearchHandle, SortField, SortSpec,
};
use mam_search::{
    AssetDoc, FilenameStrategy, Filters, IndexOptions, MediaType, Presence, SearchIndex,
};

const WORDS: [&str; 8] = [
    "sunrise", "harbor", "forest", "meeting", "concert", "portrait", "ledger", "tunnel",
];

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == &format!("--{name}"))
        .and_then(|i| a.get(i + 1).cloned())
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn row(name: &str, budget: &str, first: f64, mut v: Vec<f64>) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "| {name} | {} | {first:.1} | {:.1} / {:.1} / {:.1} | {budget} |",
        v.len(),
        pct(&v, 50.0),
        pct(&v, 95.0),
        pct(&v, 99.0)
    );
}

fn collection_of(id: u64) -> Vec<u64> {
    let mut v = Vec::new();
    if id.is_multiple_of(100) {
        v.push(1);
    }
    if id % 10 == 3 {
        v.push(2);
    }
    if id % 10 == 7 {
        v.push(3);
    }
    v
}

fn doc(id: u64) -> AssetDoc {
    let video = id.is_multiple_of(3);
    AssetDoc {
        asset_id: id,
        revision: 1,
        file_name: format!(
            "{}_{id}.{}",
            WORDS[(id % 8) as usize],
            if video { "mp4" } else { "jpg" }
        ),
        user_tags: vec![],
        ai_keywords: vec![],
        ai_description: String::new(),
        user_memo: String::new(),
        title: String::new(),
        transcript: String::new(),
        media_type: if video {
            MediaType::Video
        } else {
            MediaType::Image
        },
        volume_id: 1,
        presence: Presence::Present,
        captured_at: Some(1_600_000_000 + (id as i64 * 977) % 100_000_000),
        file_size: 1000 + id % 5000,
        duration_ms: video.then_some(1000 + id % 90_000),
        favorite: false,
        rating: 0,
        collection_ids: collection_of(id),
        tag_ids: vec![],
        modified_at: 1_600_000_000_000 + id as i64,
        folder_id: None,
        dir_key: String::new(),
        user_fields: vec![],
    }
}

fn generate(dir: &Path, n: u64) {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).unwrap();
    let t = Instant::now();
    let db = dir.join("mam.sqlite");
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
             SELECT i, CASE WHEN i % 3 = 0 THEN 'video' ELSE 'image' END, 1000 + i % 5000, i, 0, 0 FROM n"
        ),
        [n as i64],
    )
    .unwrap();
    tx.execute(
        &format!(
            "{seq} INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
             SELECT i, 1, 'd' || (i / 100) || '/f' || i || '.jpg', 'd' || (i / 100) || '/f' || i || '.jpg', 0 FROM n"
        ),
        [n as i64],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO media_metadata (asset_id, duration_ms) SELECT asset_id, 1000 + asset_id % 90000
         FROM media_assets WHERE media_type = 'video'",
        [],
    )
    .unwrap();
    // 컬렉션: 위치는 id 와 무관한 순열의 순위 × 2^20 (V12 직후와 같은 간격), 3번은 1% NULL
    for (cid, pred) in [
        (1, "asset_id % 100 = 0"),
        (2, "asset_id % 10 = 3"),
        (3, "asset_id % 10 = 7"),
    ] {
        tx.execute(
            "INSERT INTO collections (collection_id, name, name_key, created_at) VALUES (?1, ?2, ?2, 0)",
            rusqlite::params![cid, format!("c{cid}")],
        )
        .unwrap();
        tx.execute(
            &format!(
                "INSERT INTO collection_assets (collection_id, asset_id, added_at, position)
                 SELECT ?1, asset_id, asset_id,
                        CASE WHEN ?1 = 3 AND asset_id % 1000 = 7 THEN NULL
                             ELSE row_number() OVER (ORDER BY (asset_id * 2654435761) % 1000003) * 1048576 END
                 FROM media_assets WHERE {pred}"
            ),
            [cid],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    println!("sqlite: {:.1}s", t.elapsed().as_secs_f64());

    let t = Instant::now();
    let idx =
        SearchIndex::open_or_create(&dir.join("tantivy"), IndexOptions::strategy_b()).unwrap();
    for id in 1..=n {
        idx.upsert(&doc(id)).unwrap();
        if id % 200_000 == 0 {
            idx.commit().unwrap();
        }
    }
    idx.commit().unwrap();
    println!("tantivy: {:.1}s ({n} docs)", t.elapsed().as_secs_f64());
}

fn req(cid: u64, order: &Arc<CollectionOrder>, limit: usize) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters {
            collection_id: Some(cid),
            ..Filters::default()
        },
        sort: SortSpec {
            field: SortField::CollectionOrder,
            descending: false,
        },
        limit,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: Some(Arc::clone(order)),
    }
}

/// 제품 경로 한 번: 순서 맵 읽기 + 조정기 + 하이드레이션 (총 ms, 맵 ms, 검색 ms, 하이드레이션 ms, 건수)
fn page(
    qs: &QueryService,
    conn: &rusqlite::Connection,
    h: &SearchHandle,
    cid: i64,
    tweak: impl Fn(&mut CoordRequest),
) -> (f64, f64, f64, f64, usize) {
    let t0 = Instant::now();
    let order = Arc::new(CollectionOrder::from_rows(
        qs.collection_order_rows(cid).unwrap(),
    ));
    let t1 = Instant::now();
    let mut r = req(cid as u64, &order, 100);
    tweak(&mut r);
    let p = h.search(&r).unwrap();
    let t2 = Instant::now();
    let ids: Vec<i64> = p.ids.iter().map(|i| *i as i64).collect();
    let items = hydrate_page(conn, &ids).unwrap();
    let t3 = Instant::now();
    assert_eq!(items.len(), ids.len(), "hydrate dropped rows");
    (
        ms(t3 - t0),
        ms(t1 - t0),
        ms(t2 - t1),
        ms(t3 - t2),
        ids.len(),
    )
}

fn run(dir: &Path, n: u64, iters: usize) {
    let db = dir.join("mam.sqlite");
    let conn = open(&db).unwrap();
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(SystemClock);
    let qs = QueryService::new(Arc::clone(&pool), Arc::clone(&clock));
    let ud = UserData::new(Arc::clone(&pool), Arc::clone(&clock));
    let t = Instant::now();
    let idx =
        SearchIndex::open_or_create(&dir.join("tantivy"), IndexOptions::strategy_b()).unwrap();
    let h = SearchHandle::for_index(&idx);
    println!(
        "open index: {:.1}s ({n} assets)\n",
        t.elapsed().as_secs_f64()
    );
    println!("| op | runs | first (ms) | P50 / P95 / P99 (ms) | budget P95 |");
    println!("|---|---|---|---|---|");

    // 1) 첫 페이지
    for (cid, budget, name) in [
        (1, "<= 150", "first page, 10k collection"),
        (2, "<= 300", "first page, 100k collection"),
    ] {
        let first = page(&qs, &conn, &h, cid, |_| {}).0;
        let mut total = Vec::new();
        let (mut map, mut srch, mut hyd) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..iters {
            let (t, m, s, y, _) = page(&qs, &conn, &h, cid, |_| {});
            total.push(t);
            map.push(m);
            srch.push(s);
            hyd.push(y);
        }
        row(name, budget, first, total);
        for (part, v) in [("map load", map), ("tantivy", srch), ("hydrate", hyd)] {
            row(&format!("  - {part}"), "", 0.0, v);
        }
    }
    // 2) seek total/2, 3) 텍스트 질의 (100k)
    let ids_of = |cid: i64| -> Vec<i64> {
        let mut s = conn
            .prepare(
                "SELECT asset_id FROM collection_assets WHERE collection_id = ?1 ORDER BY asset_id",
            )
            .unwrap();
        s.query_map([cid], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let members = ids_of(2);
    let half = members.len() / 2;
    let seek = |r: &mut CoordRequest| r.at = Some(half);
    let first = page(&qs, &conn, &h, 2, seek).0;
    row(
        "seek at=total/2, 100k",
        "<= 400",
        first,
        (0..iters)
            .map(|_| page(&qs, &conn, &h, 2, seek).0)
            .collect(),
    );
    let q = |r: &mut CoordRequest| r.text = "harbor".into();
    let matched = {
        let order = Arc::new(CollectionOrder::from_rows(
            qs.collection_order_rows(2).unwrap(),
        ));
        let mut r = req(2, &order, 100);
        q(&mut r);
        h.search(&r).unwrap().total
    };
    let first = page(&qs, &conn, &h, 2, q).0;
    row(
        &format!("text query (matches {matched}), 100k"),
        "<= 400",
        first,
        (0..iters).map(|_| page(&qs, &conn, &h, 2, q).0).collect(),
    );

    // 4) 소속 집계 (10k 선택)
    let sel = ids_of(1);
    let first = {
        let t = Instant::now();
        ud.collection_membership(&sel).unwrap();
        ms(t.elapsed())
    };
    let mut v = Vec::new();
    for _ in 0..iters {
        let t = Instant::now();
        let m = ud.collection_membership(&sel).unwrap();
        v.push(ms(t.elapsed()));
        assert_eq!(m[0].count as usize, sel.len());
    }
    row("membership, 10k selection", "<= 100", first, v);

    // 5) 재생 대기열 (100k)
    let first = {
        let t = Instant::now();
        ud.collection_play_queue(2).unwrap();
        ms(t.elapsed())
    };
    let mut v = Vec::new();
    let mut ser = Vec::new();
    let mut bytes = 0usize;
    for _ in 0..(iters / 10).max(10) {
        let t = Instant::now();
        let qv = ud.collection_play_queue(2).unwrap();
        v.push(ms(t.elapsed()));
        let t = Instant::now();
        bytes = serde_json::to_vec(&qv).unwrap().len();
        ser.push(ms(t.elapsed()));
        assert_eq!(qv.len(), members.len());
    }
    row("play_queue, 100k (SQLite only)", "<= 500", first, v);
    row(
        &format!("  - JSON serialize ({:.1} MB)", bytes as f64 / 1e6),
        "<= 15 MB",
        0.0,
        ser,
    );

    // 5b) 진단: 같은 SQL 을 큰 페이지 캐시 연결(256 MB)로, 그리고 조인 없이 id 만 — 비용이 조인(무작위 조회)인지 확인
    {
        let big = open(&db).unwrap();
        big.pragma_update(None, "cache_size", -262_144).unwrap();
        let mm = open(&db).unwrap();
        mm.query_row("PRAGMA mmap_size = 2147483648", [], |r| r.get::<_, i64>(0))
            .unwrap();
        let both = open(&db).unwrap();
        both.pragma_update(None, "cache_size", -262_144).unwrap();
        both.query_row("PRAGMA mmap_size = 2147483648", [], |r| r.get::<_, i64>(0))
            .unwrap();
        let run_sql = |c: &rusqlite::Connection, sql: &str| -> f64 {
            let t = Instant::now();
            let mut s = c.prepare(sql).unwrap();
            let n = s.query_map([2], |r| r.get::<_, i64>(0)).unwrap().count();
            assert_eq!(n, members.len());
            ms(t.elapsed())
        };
        let ids_only = "SELECT x.asset_id FROM collection_assets x WHERE x.collection_id = ?1
                        ORDER BY x.position IS NULL, x.position, x.added_at, x.asset_id";
        for (name, c, sql) in [
            (
                "  - diag: play_queue SQL, cache 256 MB",
                &big,
                mam_core::userdata::PLAY_QUEUE_SQL,
            ),
            (
                "  - diag: play_queue SQL, mmap 2 GB only",
                &mm,
                mam_core::userdata::PLAY_QUEUE_SQL,
            ),
            (
                "  - diag: play_queue SQL, cache 256 MB + mmap",
                &both,
                mam_core::userdata::PLAY_QUEUE_SQL,
            ),
            ("  - diag: ids only (no joins)", &conn, ids_only),
        ] {
            run_sql(c, sql);
            row(name, "", 0.0, (0..10).map(|_| run_sql(c, sql)).collect());
        }
    }

    // 6) 재배치 (100k 컬렉션 2) — 간격 경로 k = 1 / 100 / 10000
    let mut rng = Lcg(11);
    // 각 묶음은 V12 직후와 같은 간격(2^20)에서 시작한다: 위치 NULL 행 하나를 만들고 k=1 이동으로 전체를 다시 번호 매긴다.
    // 마지막 묶음은 간격이 이미 쪼개진 상태의 k=10000 (간격 부족 -> 전체 재작성 폴백이 섞인다)
    let fresh = |salt: usize| {
        conn.execute(
            "UPDATE collection_assets SET position = NULL WHERE collection_id = 2 AND asset_id = ?1",
            [members[salt]],
        )
        .unwrap();
        ud.reorder_collection(2, &[members[salt + 1]], ReorderTarget::End)
            .unwrap();
    };
    for (k, budget, runs, label, reset) in [
        (1usize, "<= 30", iters, "gap path", 1u8),
        (100, "<= 50", iters, "gap path", 1),
        (
            10_000,
            "<= 300",
            30,
            "gap path, gaps reset before each run",
            2,
        ),
        (10_000, "(fallback mix)", 30, "fragmented gaps", 0),
    ] {
        if reset == 1 {
            fresh(k % 1000);
        }
        let mut v = Vec::new();
        for r in 0..runs {
            if reset == 2 {
                fresh(k % 1000);
            }
            let start = (rng.next() as usize) % (members.len() - k);
            let moved: Vec<i64> = (0..k)
                .map(|j| members[(start + j * 7) % members.len()])
                .collect();
            let mut anchor = members[(rng.next() as usize) % members.len()];
            while moved.contains(&anchor) {
                anchor = members[(rng.next() as usize) % members.len()];
            }
            let t = Instant::now();
            ud.reorder_collection(2, &moved, ReorderTarget::Before { asset_id: anchor })
                .unwrap();
            let d = ms(t.elapsed());
            if r > 0 || runs == 30 {
                v.push(d);
            }
        }
        row(&format!("reorder {label} k={k}, 100k"), budget, 0.0, v);
    }
    // 7) 강제 rebalance: 위치 NULL 행을 하나 만들고 k=1 이동 -> normalize_if_null -> 전체 번호 재부여
    let mut v = Vec::new();
    for i in 0..20 {
        conn.execute(
            "UPDATE collection_assets SET position = NULL WHERE collection_id = 2 AND asset_id = ?1",
            [members[i * 5]],
        )
        .unwrap();
        let t = Instant::now();
        ud.reorder_collection(2, &[members[i * 5 + 1]], ReorderTarget::End)
            .unwrap();
        v.push(ms(t.elapsed()));
    }
    row(
        "forced rebalance (normalize NULL) + move, 100k",
        "<= 1500",
        0.0,
        v,
    );

    // 8) 위치 NULL 1% 컬렉션 (3) 첫 페이지 — NULL 등급 포함
    let first = page(&qs, &conn, &h, 3, |_| {}).0;
    let v = (0..iters)
        .map(|_| page(&qs, &conn, &h, 3, |_| {}).0)
        .collect();
    row(
        "first page, 100k with 1% NULL positions",
        "<= 300",
        first,
        v,
    );
}

/// 실제 100만 건 DB 에서의 쿼리 계획 (EXPLAIN QUERY PLAN)
fn plans(dir: &Path) {
    let conn = open(&dir.join("mam.sqlite")).unwrap();
    let show = |name: &str, sql: &str, p: &[&dyn rusqlite::ToSql]| {
        let mut s = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let lines: Vec<String> = s
            .query_map(p, |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        println!("{name}:\n  {}", lines.join("\n  "));
    };
    show("play_queue", mam_core::userdata::PLAY_QUEUE_SQL, &[&2i64]);
    show(
        "membership",
        mam_core::userdata::MEMBERSHIP_SQL,
        &[&"[100,200]"],
    );
    show(
        "neighbour (before)",
        "SELECT max(position) FROM collection_assets WHERE collection_id = 2 AND position < 5000000000
           AND asset_id NOT IN (SELECT value FROM json_each('[3,13]'))",
        &[],
    );
    show(
        "collection_order_rows",
        "SELECT asset_id, position, added_at FROM collection_assets WHERE collection_id = ?1",
        &[&2i64],
    );
    show(
        "write_positions (per row)",
        "UPDATE collection_assets SET position = 1 WHERE collection_id = 2 AND asset_id = 3",
        &[],
    );
    show(
        "rebalance_with_block (fallback)",
        "UPDATE collection_assets SET position = r.rn * 1048576
         FROM (SELECT asset_id, row_number() OVER (ORDER BY k, m, position, asset_id) AS rn
                 FROM (SELECT asset_id, position,
                              CASE WHEN asset_id IN (SELECT value FROM json_each(?1)) THEN 5000 ELSE position END AS k,
                              CASE WHEN asset_id IN (SELECT value FROM json_each(?1)) THEN 0 ELSE 1 END AS m
                         FROM collection_assets WHERE collection_id = 2)) AS r
         WHERE collection_assets.collection_id = 2 AND collection_assets.asset_id = r.asset_id",
        &[&"[3,13]"],
    );
}

fn main() {
    let dir = PathBuf::from(arg("dir").expect("--dir"));
    let n: u64 = arg("assets")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = arg("iters").and_then(|v| v.parse().ok()).unwrap_or(200);
    match arg("phase").as_deref() {
        Some("gen") => generate(&dir, n),
        Some("run") => run(&dir, n, iters),
        Some("plan") => plans(&dir),
        _ => panic!("--phase gen|run"),
    }
}
