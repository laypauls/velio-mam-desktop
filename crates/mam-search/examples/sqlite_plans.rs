//! ANALYZE 된 데이터셋에서 `EXPLAIN QUERY PLAN` 재확인 + 후보 50건 상세 조회 시간.
//!
//! 실행: `cargo run --release -p mam-search --example sqlite_plans -- --size 10000`
//! 옵션: `--runs 1000`, `--candidates 50`, `--seed 11`, `--out <dir>`
//!
//! 플랜 대상 질의는 `crates/mam-db/tests/query_plans.rs` 와 같다 (+ 재구축 질의, 상세 조회).

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::rng::Rng;
use common::stats::summarize;
use common::{args, json_str, load, sysinfo};
use rusqlite::{Connection, params, params_from_iter};

fn plan(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("explain");
    let nulls = vec![rusqlite::types::Null; stmt.parameter_count()];
    stmt.query_map(params_from_iter(nulls), |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

const DETAIL_SELECT: &str =
    "SELECT a.asset_id, a.media_type, a.status, a.file_size, a.modified_at, a.revision,
        l.volume_id, l.rel_path, l.presence, v.kind, v.availability, v.label,
        m.width, m.height, m.duration_ms, m.codec, m.captured_at,
        (SELECT group_concat(t.name, char(31)) FROM asset_tags at
           JOIN tags t ON t.tag_id = at.tag_id WHERE at.asset_id = a.asset_id)
 FROM media_assets a
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 JOIN volumes v ON v.volume_id = l.volume_id
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id";

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let runs: usize = a.num("runs", 1000);
    let k: usize = a.num("candidates", 50);
    let mut rng = Rng::new(a.num("seed", 11));
    let layout = common::Layout::new(size, a.path("out"));
    let conn = common::open_db(&layout.db());
    let ds_seed = common::setting(&conn, "poc.seed").unwrap_or_default();
    let stat_rows: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_stat1", [], |r| r.get(0))
        .unwrap_or(0);

    let in_list = (1..=k)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let detail_in = format!("{DETAIL_SELECT} WHERE a.asset_id IN ({in_list})");
    let detail_one = format!("{DETAIL_SELECT} WHERE a.asset_id = ?1");
    let queries: Vec<(&str, String)> = vec![
        ("path lookup", "SELECT asset_id FROM file_locations WHERE volume_id = 1 AND path_key = 'a/b.jpg' AND detached_at IS NULL".into()),
        ("hash lookup", "SELECT asset_id FROM media_assets WHERE hash_algo = 'blake3' AND content_hash = x'00112233445566778899aabbccddeeff'".into()),
        ("locations by asset", "SELECT location_id, volume_id, path_key FROM file_locations WHERE asset_id = 7".into()),
        ("pending hash scan", "SELECT asset_id FROM media_assets WHERE hash_state = 'pending' ORDER BY asset_id LIMIT 100".into()),
        ("outbox pending scan", "SELECT event_id, asset_id, revision FROM search_outbox WHERE status = 'pending' ORDER BY priority, event_id LIMIT 100".into()),
        ("type/status filter", "SELECT asset_id FROM media_assets WHERE media_type = 'image' AND status = 'active' LIMIT 100".into()),
        ("locations by volume", "SELECT location_id FROM file_locations WHERE volume_id = 3".into()),
        ("relations by target", "SELECT from_asset_id FROM asset_relations WHERE to_asset_id = 3".into()),
        ("(control) unindexed file_size", "SELECT asset_id FROM media_assets WHERE file_size = 10".into()),
        ("(expected scan) online volumes (D2), tiny table", "SELECT volume_id FROM volumes WHERE availability = 'online'".into()),
        ("detail fetch IN (...)", detail_in.clone()),
        ("detail fetch single id", detail_one.clone()),
        ("(expected scan) index rebuild stream, full pass", load::ALL_DOCS_SQL.to_string()),
    ];

    let mut plan_rows = Vec::new();
    let mut scans = 0;
    for (name, sql) in &queries {
        let p = plan(&conn, sql);
        let scan = p.iter().any(|d| d.starts_with("SCAN"));
        let temp = p.iter().any(|d| d.contains("TEMP B-TREE"));
        plan_rows.push((name.to_string(), p, scan, temp));
        if scan && !is_expected(name) {
            scans += 1;
        }
    }

    // 상세 조회: 무작위 후보 k건 (실행마다 다른 id 집합)
    let mut id_sets: Vec<Vec<i64>> = Vec::with_capacity(runs + 50);
    for _ in 0..runs + 50 {
        id_sets.push((0..k).map(|_| rng.range(1, size) as i64).collect());
    }
    let mut stmt_in = conn.prepare_cached(&detail_in).unwrap();
    let mut stmt_one = conn.prepare_cached(&detail_one).unwrap();
    let fetch_in = |stmt: &mut rusqlite::CachedStatement, ids: &[i64]| -> usize {
        let mut rows = stmt.query(params_from_iter(ids.iter())).unwrap();
        let mut n = 0;
        while let Some(r) = rows.next().unwrap() {
            let _ = (r.get::<_, i64>(0).unwrap(), r.get::<_, String>(7).unwrap());
            let _ = r.get::<_, Option<String>>(17).unwrap();
            n += 1;
        }
        n
    };
    for ids in id_sets.iter().take(50) {
        fetch_in(&mut stmt_in, ids);
    }
    let mut in_samples = Vec::with_capacity(runs);
    let mut short = 0;
    for ids in id_sets.iter().skip(50) {
        let t = Instant::now();
        let n = fetch_in(&mut stmt_in, ids);
        in_samples.push(t.elapsed());
        // 무작위 id 에 중복이 있으면 행 수가 k 보다 작다 — 고유 id 수와 비교한다
        let mut u = ids.clone();
        u.sort_unstable();
        u.dedup();
        if n != u.len() {
            short += 1;
        }
    }
    // 대조: N+1 (앱에서는 금지) — 같은 id 집합을 k 번 왕복
    let n1_runs = runs.min(300);
    let mut n1_samples: Vec<Duration> = Vec::with_capacity(n1_runs);
    for ids in id_sets.iter().skip(50).take(n1_runs) {
        let t = Instant::now();
        for id in ids {
            stmt_one.query_row(params![id], |r| r.get::<_, i64>(0)).ok();
        }
        n1_samples.push(t.elapsed());
    }
    let s_in = summarize(&in_samples);
    let s_n1 = summarize(&n1_samples);

    let cond = sysinfo::conditions();
    println!("\n== sqlite_plans ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: size={size} dataset_seed={ds_seed}; ANALYZEd: sqlite_stat1 rows={stat_rows}"
    );
    println!("\n### EXPLAIN QUERY PLAN (SCAN flagged)\n");
    for (name, p, scan, temp) in &plan_rows {
        let flag = match (scan, temp) {
            (true, _) if is_expected(name) => "SCAN (expected)",
            (true, true) => "SCAN + TEMP B-TREE",
            (true, false) => "SCAN",
            (false, true) => "TEMP B-TREE (sort)",
            _ => "ok",
        };
        println!("- **{name}** [{flag}]");
        for d in p {
            println!("    - {d}");
        }
    }
    println!("\nSCAN count (excluding control): {scans}");
    println!(
        "\n### Detail fetch for {k} random candidate asset_ids ({runs} runs, warm page cache)\n"
    );
    println!(
        "| method | runs | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|"
    );
    println!(
        "| one batched query, IN ({k} ids) | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        s_in.n, s_in.p50_ms, s_in.p95_ms, s_in.p99_ms, s_in.max_ms
    );
    println!(
        "| contrast: {k} round trips (N+1, forbidden) | {} | {:.3} | {:.3} | {:.3} | {:.3} |",
        s_n1.n, s_n1.p50_ms, s_n1.p95_ms, s_n1.p99_ms, s_n1.max_ms
    );
    if short > 0 {
        println!("WARNING: {short} runs returned fewer rows than distinct ids");
    }
    println!("(page cache not flushed; ids are random over all {size} assets)");

    let plans_json: Vec<String> = plan_rows
        .iter()
        .map(|(n, p, scan, temp)| {
            format!(
                "{{\"name\":{},\"scan\":{scan},\"temp_btree\":{temp},\"plan\":[{}]}}",
                json_str(n),
                p.iter().map(|d| json_str(d)).collect::<Vec<_>>().join(",")
            )
        })
        .collect();
    println!(
        "JSON: {{\"tool\":\"sqlite_plans\",\"size\":{size},\"dataset_seed\":{},\"stat1_rows\":{stat_rows},\"scans_excluding_control\":{scans},\"detail_in\":{},\"detail_n_plus_1\":{},\"plans\":[{}],\"conditions\":{}}}",
        json_str(&ds_seed),
        s_in.json(),
        s_n1.json(),
        plans_json.join(","),
        common::conditions_json(&cond)
    );
}

/// 대조군이거나 의도된 전체 스캔(작은 테이블·전체 재구축 패스)인가.
fn is_expected(name: &str) -> bool {
    name.starts_with("(control)") || name.starts_with("(expected scan)")
}
