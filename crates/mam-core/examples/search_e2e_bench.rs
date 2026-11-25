//! P2-5 종단 지연 측정 — 검색 조정기(Tantivy) + 페이지 하이드레이션(SQLite) + 상세 조회.
//!
//! 실행: `cargo run --release -p mam-core --example search_e2e_bench -- --db <dataset.db> --index <index dir> [--iters 500]`
//! 데이터셋은 `mam-search` 의 `gen_dataset` 이 만든다 (그 DB 에 관리 폴더 1개를 추가한다).
//! 인덱스 디렉터리가 없으면 제품 `SyncWorker::open` 이 SQLite 에서 전체 재구축한다 (그 시간도 출력).
//! 보고 단위: 시나리오별 P50/P95/P99 (ms) — 총 지연과 그중 Tantivy / 하이드레이션.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mam_core::query::{asset_detail, hydrate_page};
use mam_search::coordinator::{CoordRequest, Direction, SearchHandle, SortField, SortSpec};
use mam_search::search::FolderFilter;
use mam_search::sync::{SyncConfig, SyncWorker};
use mam_search::{FilenameStrategy, Filters, IndexOptions, MediaType, Presence};
use rusqlite::Connection;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[(self.next() as usize) % v.len()]
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

#[derive(Default)]
struct Samples {
    total: Vec<f64>,
    search: Vec<f64>,
    hydrate: Vec<f64>,
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn report(name: &str, s: &mut Samples) {
    for v in [&mut s.total, &mut s.search, &mut s.hydrate] {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    println!(
        "| {name} | {} | {:.1} / {:.1} / {:.1} | {:.1} / {:.1} / {:.1} | {:.1} / {:.1} / {:.1} |",
        s.total.len(),
        pct(&s.total, 50.0),
        pct(&s.total, 95.0),
        pct(&s.total, 99.0),
        pct(&s.search, 50.0),
        pct(&s.search, 95.0),
        pct(&s.search, 99.0),
        pct(&s.hydrate, 50.0),
        pct(&s.hydrate, 95.0),
        pct(&s.hydrate, 99.0),
    );
}

/// 파일명(확장자 제외)에서 3글자 이상 영숫자·한글 토큰을 뽑는다.
fn tokens(rel_path: &str) -> Vec<String> {
    let name = rel_path.rsplit('/').next().unwrap_or("");
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    stem.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 3 && !t.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

fn run_page(
    conn: &Connection,
    handle: &SearchHandle,
    req: &CoordRequest,
    s: &mut Samples,
    warm: bool,
) -> Option<String> {
    let t0 = Instant::now();
    let page = handle.search(req).expect("search");
    let t1 = Instant::now();
    let ids: Vec<i64> = page.ids.iter().map(|i| *i as i64).collect();
    let items = hydrate_page(conn, &ids).expect("hydrate");
    let t2 = Instant::now();
    assert!(items.len() <= 50);
    if warm {
        s.total.push(ms(t2 - t0));
        s.search.push(ms(t1 - t0));
        s.hydrate.push(ms(t2 - t1));
    }
    page.next_cursor
}

fn main() {
    let db = PathBuf::from(arg("db").expect("--db"));
    let index_dir = PathBuf::from(arg("index").expect("--index"));
    let iters: usize = arg("iters").and_then(|v| v.parse().ok()).unwrap_or(500);
    let warmup = 50usize;
    let conn = mam_db::open(&db).expect("open db");
    let mut rng = Lcg(7);

    // 관리 폴더 1개: 샘플에서 가장 많은 (볼륨, 최상위 폴더)
    let mut counts: BTreeMap<(i64, String), u32> = BTreeMap::new();
    let mut names: Vec<String> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT volume_id, rel_path FROM file_locations WHERE asset_id % 97 = 0 LIMIT 12000")
            .unwrap();
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        for row in rows {
            let (v, p) = row.unwrap();
            let root = p.split('/').next().unwrap_or("").to_lowercase();
            *counts.entry((v, root)).or_default() += 1;
            names.push(p);
        }
    }
    let ((folder_vol, folder_root), _) = counts.iter().max_by_key(|(_, c)| **c).unwrap();
    let have: i64 = conn
        .query_row("SELECT count(*) FROM managed_folders", [], |r| r.get(0))
        .unwrap();
    if have == 0 {
        conn.execute(
            "INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at)
             VALUES (1, ?1, ?2, ?2, ?2, 0)",
            rusqlite::params![folder_vol, folder_root],
        )
        .unwrap();
    }

    let t = Instant::now();
    let (worker, outcome) = SyncWorker::open(
        &conn,
        &index_dir,
        IndexOptions::strategy_b(),
        SyncConfig::default(),
    )
    .expect("open worker");
    println!(
        "open: {:.1}s, rebuilt docs: {:?}",
        t.elapsed().as_secs_f64(),
        outcome.rebuilt.as_ref().map(|r| r.docs)
    );
    let handle = worker.search_handle();

    let online: Vec<u64> = conn
        .prepare("SELECT volume_id FROM volumes WHERE availability = 'online'")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|v| v.unwrap() as u64)
        .collect();
    let tag_max: i64 = conn
        .query_row("SELECT max(tag_id) FROM tags", [], |r| r.get(0))
        .unwrap();
    let words: Vec<String> = names.iter().flat_map(|n| tokens(n)).collect();

    let base = || CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters {
            presence: Some(Presence::Present),
            ..Default::default()
        },
        sort: SortSpec::default(),
        limit: 50,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    };
    let sorted = |f, desc| SortSpec {
        field: f,
        descending: desc,
    };

    println!(
        "\n| scenario | n | total P50 / P95 / P99 ms | tantivy | hydrate |\n|---|---|---|---|---|"
    );
    type Build<'a> = Box<dyn FnMut(&mut Lcg) -> CoordRequest + 'a>;
    let scenarios: Vec<(&str, Build)> = vec![
        (
            "S1 keyword, relevance, default filters",
            Box::new(|r: &mut Lcg| CoordRequest {
                text: r.pick(&words).clone(),
                ..base()
            }),
        ),
        (
            "S2 keyword + video + hide offline",
            Box::new(|r: &mut Lcg| {
                let mut q = base();
                q.text = r.pick(&words).clone();
                q.filters.media_types = vec![MediaType::Video];
                q.filters.online_volume_ids = Some(online.clone());
                q
            }),
        ),
        (
            "S3 keyword + folder (recursive), captured desc",
            Box::new(|r: &mut Lcg| {
                let mut q = base();
                q.text = r.pick(&words).clone();
                q.filters.folder = Some(FolderFilter {
                    folder_id: 1,
                    dir_key: String::new(),
                    recursive: true,
                });
                q.sort = sorted(SortField::CapturedAt, true);
                q
            }),
        ),
        (
            "S4 browse, captured desc",
            Box::new(|_: &mut Lcg| {
                let mut q = base();
                q.sort = sorted(SortField::CapturedAt, true);
                q
            }),
        ),
        (
            "S5 browse, name asc",
            Box::new(|_: &mut Lcg| {
                let mut q = base();
                q.sort = sorted(SortField::Name, false);
                q
            }),
        ),
        (
            "S6 browse + 1 tag, modified desc",
            Box::new(|r: &mut Lcg| {
                let mut q = base();
                q.filters.tag_ids = vec![(r.next() as i64 % tag_max + 1) as u64];
                q.sort = sorted(SortField::ModifiedAt, true);
                q
            }),
        ),
    ];
    for (name, mut build) in scenarios {
        let mut s = Samples::default();
        for i in 0..(iters + warmup) {
            let req = build(&mut rng);
            run_page(&conn, &handle, &req, &mut s, i >= warmup);
        }
        report(name, &mut s);
    }

    // S7: 깊은 페이지 — 촬영일 내림차순 탐색을 커서로 20페이지 걷는다 (2~20 페이지 지연)
    let mut s = Samples::default();
    for round in 0..(iters / 20).max(5) {
        let mut req = base();
        req.sort = sorted(SortField::CapturedAt, true);
        for page_no in 0..20 {
            let cur = run_page(&conn, &handle, &req, &mut s, page_no > 0 && round > 0);
            req.cursor = cur;
            if req.cursor.is_none() {
                break;
            }
        }
    }
    report("S7 browse captured desc, pages 2-20 via cursor", &mut s);

    // S8: 상세 조회
    let ids: Vec<i64> = (0..iters + warmup)
        .map(|_| (rng.next() % 1_000_000) as i64 + 1)
        .collect();
    let mut s = Samples::default();
    for (i, id) in ids.iter().enumerate() {
        let t = Instant::now();
        let _ = asset_detail(&conn, *id);
        if i >= warmup {
            s.total.push(ms(t.elapsed()));
        }
    }
    s.total.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "| S8 get_asset_detail | {} | {:.2} / {:.2} / {:.2} | - | - |",
        s.total.len(),
        pct(&s.total, 50.0),
        pct(&s.total, 95.0),
        pct(&s.total, 99.0)
    );
}
