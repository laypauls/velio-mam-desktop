//! 질의 지연 측정 — 유형별 P50/P95/P99, 콜드(첫 질의)·웜, 단건 갱신 반영 지연, D2 볼륨 필터 비용.
//!
//! 실행: `cargo run --release -p mam-search --example bench_query -- --size 10000 --strategy a`
//! 옵션: `--runs 1000`(유형당 측정 횟수), `--warmup 100`, `--cap-secs 120`(유형당 시간 상한 —
//! 상한에 걸리면 실제 횟수를 표시), `--updates 30`, `--seed 7`(질의 표본), `--out <dir>`
//!
//! "콜드" = 새 프로세스에서 인덱스를 연 직후의 첫 질의. **OS 파일 캐시는 비우지 않는다**
//! (진짜 콜드는 관리자 권한 RAMMap/EmptyStandbyList 필요 — 시도하지 않음).
//! 주의: 단건 갱신 측정은 데이터셋 SQLite 에 태그/revision 변경을 실제로 기록한다.

#[path = "common/mod.rs"]
mod common;

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use common::rng::Rng;
use common::stats::{Summary, summarize};
use common::vocab::{ALL_PARTICLES, FN_EN, FN_KO, FN_PLACES, Josa, TEXT_EN, TEXT_KO, with_josa};
use common::{args, json_str, load, sysinfo};
use mam_search::{
    FilenameStrategy, Filters, IndexOptions, MediaType, Range, SearchIndex, SearchRequest, Sort,
    UpsertOutcome,
};
use rusqlite::{Connection, params};

/// 2024-01-01 / 2025-01-01 00:00 UTC (유닉스 초).
const Y2024: (i64, i64) = (1_704_067_200, 1_735_689_599);

struct TypeResult {
    name: String,
    distinct: usize,
    first_ms: f64,
    s: Summary,
    mean_hits: f64,
    capped: bool,
}

fn req(text: &str, strategy: FilenameStrategy) -> SearchRequest {
    SearchRequest {
        text: text.to_string(),
        strategy,
        ..Default::default()
    }
}

fn run_type(
    index: &SearchIndex,
    name: &str,
    reqs: &[SearchRequest],
    runs: usize,
    warmup: usize,
    cap: Duration,
) -> TypeResult {
    if reqs.is_empty() {
        args::fail(&format!("query set '{name}' is empty"));
    }
    let t = Instant::now();
    index.search(&reqs[0]).expect("search");
    let first_ms = t.elapsed().as_secs_f64() * 1000.0;
    for i in 0..warmup {
        index.search(&reqs[i % reqs.len()]).expect("warmup");
    }
    let mut samples = Vec::with_capacity(runs);
    let mut hits = 0usize;
    let started = Instant::now();
    for i in 0..runs {
        let r = &reqs[i % reqs.len()];
        let t = Instant::now();
        let h = index.search(r).expect("search");
        samples.push(t.elapsed());
        hits += h.len();
        if started.elapsed() > cap {
            break;
        }
    }
    TypeResult {
        name: name.to_string(),
        distinct: reqs.len(),
        first_ms,
        capped: samples.len() < runs,
        mean_hits: hits as f64 / samples.len().max(1) as f64,
        s: summarize(&samples),
    }
}

fn dedupe(mut v: Vec<String>, rng: &mut Rng, want: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
    // 결정적 셔플 후 상한
    for i in (1..v.len()).rev() {
        v.swap(i, rng.below(i + 1));
    }
    v.truncate(want);
    v
}

fn sample_paths(conn: &Connection, rng: &mut Rng, size: u64, n: usize) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT rel_path FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL")
        .unwrap();
    (0..n)
        .filter_map(|_| {
            let id = rng.range(1, size) as i64;
            stmt.query_row(params![id], |r| r.get::<_, String>(0)).ok()
        })
        .map(|p| p.rsplit('/').next().unwrap_or("").to_string())
        .collect()
}

/// 파일명의 숫자 덩어리(4자리 이상)에서 4자리 조각.
fn digit_queries(names: &[String], rng: &mut Rng) -> Vec<String> {
    let mut out = Vec::new();
    for n in names {
        let mut run = String::new();
        let mut runs = Vec::new();
        for c in n.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_digit() {
                run.push(c);
            } else {
                if run.len() >= 4 {
                    runs.push(std::mem::take(&mut run));
                }
                run.clear();
            }
        }
        if let Some(r) = runs.get(rng.below(runs.len().max(1))) {
            let off = rng.below(r.len() - 3);
            out.push(r[off..off + 4].to_string());
        }
    }
    out
}

fn ko2_queries() -> Vec<String> {
    let mut v: Vec<String> = FN_PLACES.iter().map(|s| s.to_string()).collect();
    for w in FN_KO {
        let c: Vec<char> = w.chars().collect();
        for win in c.windows(2) {
            v.push(win.iter().collect());
        }
    }
    v
}

fn en_queries(rng: &mut Rng) -> Vec<String> {
    let mut v = Vec::new();
    for w in FN_EN {
        v.push(w.to_string());
        let c: Vec<char> = w.chars().collect();
        for _ in 0..3 {
            if c.len() > 3 {
                let len = rng.range(3, c.len() as u64 - 1) as usize;
                let off = rng.below(c.len() - len + 1);
                v.push(c[off..off + len].iter().collect());
            }
        }
    }
    v
}

fn lemma_queries() -> Vec<String> {
    TEXT_KO
        .iter()
        .chain(TEXT_EN)
        .map(|s| s.to_string())
        .collect()
}

fn particle_queries(rng: &mut Rng) -> Vec<String> {
    let mut v = Vec::new();
    for l in TEXT_KO {
        v.push(with_josa(
            l,
            [Josa::Subject, Josa::Object, Josa::With, Josa::Topic][rng.below(4)],
        ));
        v.push(format!("{l}{}", rng.pick(ALL_PARTICLES)));
    }
    v
}

fn base36(mut n: u128) -> String {
    let mut s = String::new();
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
    }
    s
}

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let (letter, strategy, opts) = common::parse_strategy(a.get("strategy").unwrap_or("a"));
    let runs: usize = a.num("runs", 1000);
    let warmup: usize = a.num("warmup", 100);
    let cap = Duration::from_secs(a.num("cap-secs", 120));
    let updates: usize = a.num("updates", 30);
    let mut rng = Rng::new(a.num("seed", 7));
    let layout = common::Layout::new(size, a.path("out"));
    let index_dir = layout.index(letter);
    if !index_dir.exists() {
        args::fail("index not found - run build_index first");
    }
    let conn = common::open_db(&layout.db());
    let ds_seed = common::setting(&conn, "poc.seed").unwrap_or_default();

    // 질의 표본 (SQLite 읽기는 측정 구간 밖)
    let names = sample_paths(&conn, &mut rng, size, 1200);
    let exact: Vec<String> = dedupe(
        names
            .iter()
            .filter(|n| {
                n.rsplit_once('.')
                    .is_some_and(|(s, _)| s.chars().count() >= 4)
            })
            .cloned()
            .collect(),
        &mut rng,
        200,
    );
    let digits = dedupe(digit_queries(&names, &mut rng), &mut rng, 200);
    let ko2 = dedupe(ko2_queries(), &mut rng, 200);
    let en = dedupe(en_queries(&mut rng), &mut rng, 200);
    let lemma = dedupe(lemma_queries(), &mut rng, 200);
    let particle = dedupe(particle_queries(&mut rng), &mut rng, 200);

    let t = Instant::now();
    let online: Vec<u64> = conn
        .prepare("SELECT volume_id FROM volumes WHERE availability = 'online' ORDER BY volume_id")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|v| v.unwrap() as u64)
        .collect();
    let online_read_us = t.elapsed().as_secs_f64() * 1e6;

    // 인덱스 열기 (콜드 측정의 시작)
    let t_open = Instant::now();
    let index = SearchIndex::open_or_create(&index_dir, opts.clone()).expect("open index");
    let open_ms = t_open.elapsed().as_secs_f64() * 1000.0;
    let segments = common::segment_count(&index_dir);
    let index_bytes = common::dir_size(&index_dir);

    let plain =
        |qs: &[String]| -> Vec<SearchRequest> { qs.iter().map(|q| req(q, strategy)).collect() };
    let with_online = |qs: &[String]| -> Vec<SearchRequest> {
        qs.iter()
            .map(|q| {
                let mut r = req(q, strategy);
                r.filters.online_volume_ids = Some(online.clone());
                r
            })
            .collect()
    };
    let composite: Vec<SearchRequest> = lemma
        .iter()
        .map(|q| SearchRequest {
            filters: Filters {
                media_types: vec![MediaType::Video],
                online_volume_ids: Some(online.clone()),
                captured_at: Range {
                    min: Some(Y2024.0),
                    max: Some(Y2024.1),
                },
                ..Default::default()
            },
            ..req(q, strategy)
        })
        .collect();
    let filter_only = vec![SearchRequest {
        filters: composite[0].filters.clone(),
        ..req("", strategy)
    }];
    let by_captured: Vec<SearchRequest> = lemma
        .iter()
        .map(|q| SearchRequest {
            sort: Sort::CapturedAtDesc,
            ..req(q, strategy)
        })
        .collect();

    let sets: Vec<(&str, Vec<SearchRequest>)> = vec![
        ("filename_exact", plain(&exact)),
        ("filename_partial_ko2", plain(&ko2)),
        ("filename_partial_en", plain(&en)),
        ("filename_partial_digits", plain(&digits)),
        ("fulltext_plain", plain(&lemma)),
        ("fulltext_particle", plain(&particle)),
        ("composite_video_2024_online_kw", composite),
        ("filter_only_video_2024_online", filter_only),
        ("sort_relevance", plain(&lemma)),
        ("sort_captured_at_desc", by_captured),
        ("d2_fulltext_no_volume_filter", plain(&lemma)),
        ("d2_fulltext_online_volume_filter", with_online(&lemma)),
        ("d2_fn_ko2_no_volume_filter", plain(&ko2)),
        ("d2_fn_ko2_online_volume_filter", with_online(&ko2)),
    ];
    let results: Vec<TypeResult> = sets
        .iter()
        .map(|(n, r)| {
            eprintln!("  running {n} ({} distinct queries)", r.len());
            run_type(&index, n, r, runs, warmup, cap)
        })
        .collect();

    // 단건 갱신: 태그 변경(+revision 증가) -> upsert -> commit -> 새 태그 검색 반영
    let mut slot = Some(index);
    let upd = single_updates(
        &conn, &mut slot, &index_dir, &opts, strategy, size, updates, &mut rng,
    );

    let peak = sysinfo::peak_rss_bytes();
    let cond = sysinfo::conditions();
    println!("\n== bench_query ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: size={size} dataset_seed={ds_seed} query_seed(arg)=see --seed strategy={} | index: segments={segments} (merged state as built), {:.1} MB",
        letter.to_ascii_uppercase(),
        sysinfo::mb(index_bytes)
    );
    println!(
        "Open index: {open_ms:.1} ms | online volumes: {} (read from SQLite in {online_read_us:.0} us)",
        online.len()
    );
    println!(
        "Page: first 50 hits, offset 0. warmup={warmup}, target runs={runs}, per-type cap={}s",
        cap.as_secs()
    );
    println!("Cold = first query of that type in this process (OS file cache NOT flushed).\n");
    println!(
        "| query type | distinct | runs | first ms | P50 ms | P95 ms | P99 ms | max ms | mean hits |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for r in &results {
        println!(
            "| {} | {} | {}{} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.1} |",
            r.name,
            r.distinct,
            r.s.n,
            if r.capped { " (CAPPED)" } else { "" },
            r.first_ms,
            r.s.p50_ms,
            r.s.p95_ms,
            r.s.p99_ms,
            r.s.max_ms,
            r.mean_hits
        );
    }
    let find = |n: &str| results.iter().find(|r| r.name == n).unwrap().s.p50_ms;
    println!(
        "\nD2 volume-filter overhead (P50): fulltext {:.2} -> {:.2} ms ({:+.2}), fn_ko2 {:.2} -> {:.2} ms ({:+.2})",
        find("d2_fulltext_no_volume_filter"),
        find("d2_fulltext_online_volume_filter"),
        find("d2_fulltext_online_volume_filter") - find("d2_fulltext_no_volume_filter"),
        find("d2_fn_ko2_no_volume_filter"),
        find("d2_fn_ko2_online_volume_filter"),
        find("d2_fn_ko2_online_volume_filter") - find("d2_fn_ko2_no_volume_filter"),
    );
    println!(
        "\nSingle-update latency ({} runs; tag change + revision bump):",
        upd.n
    );
    println!("| phase | P50 ms | P95 ms | max ms |\n|---|---:|---:|---:|");
    for (n, s) in [
        ("SQLite txn (tag+revision) [not in latency]", &upd.sqlite),
        ("upsert (guarded)", &upd.upsert),
        ("commit (+reader reload)", &upd.commit),
        ("search sees new tag", &upd.search),
        ("TOTAL upsert..searchable", &upd.total),
    ] {
        println!(
            "| {n} | {:.2} | {:.2} | {:.2} |",
            s.p50_ms, s.p95_ms, s.max_ms
        );
    }
    if upd.commit_errors > 0 {
        println!(
            "WARNING: {} commit error(s) (os error 5 PermissionDenied on a new segment file); index was closed and reopened to recover",
            upd.commit_errors
        );
    }
    if upd.stale > 0 {
        println!("WARNING: {} upserts were rejected as stale", upd.stale);
    }
    println!(
        "Index num_docs before/after update phase: {}/{} (must be equal; updates replace docs)",
        upd.docs_before, upd.docs_after
    );
    if upd.misses > 0 {
        println!(
            "WARNING: {} updates were not visible on the first search",
            upd.misses
        );
    }
    println!("\nPeak RSS (working set): {:.0} MB", sysinfo::mb(peak));

    let rows: Vec<String> = results
        .iter()
        .map(|r| {
            format!(
                "{{\"type\":{},\"distinct\":{},\"first_ms\":{:.3},\"capped\":{},\"mean_hits\":{:.2},\"latency\":{}}}",
                json_str(&r.name), r.distinct, r.first_ms, r.capped, r.mean_hits, r.s.json()
            )
        })
        .collect();
    println!(
        "JSON: {{\"tool\":\"bench_query\",\"size\":{size},\"strategy\":\"{}\",\"dataset_seed\":{},\"segments\":{segments},\"index_bytes\":{index_bytes},\"open_ms\":{open_ms:.1},\"peak_rss_bytes\":{peak},\"results\":[{}],\"update\":{{\"n\":{},\"misses\":{},\"commit_errors\":{},\"stale\":{},\"docs_before\":{},\"docs_after\":{},\"upsert\":{},\"commit\":{},\"search\":{},\"total\":{},\"sqlite\":{}}},\"conditions\":{}}}",
        letter,
        json_str(&ds_seed),
        rows.join(","),
        upd.n,
        upd.misses,
        upd.commit_errors,
        upd.stale,
        upd.docs_before,
        upd.docs_after,
        upd.upsert.json(),
        upd.commit.json(),
        upd.search.json(),
        upd.total.json(),
        upd.sqlite.json(),
        common::conditions_json(&cond)
    );
}

struct UpdateStats {
    n: usize,
    misses: usize,
    commit_errors: usize,
    stale: usize,
    docs_before: u64,
    docs_after: u64,
    sqlite: Summary,
    upsert: Summary,
    commit: Summary,
    search: Summary,
    total: Summary,
}

/// commit 이 실패한 뒤의 `SearchIndex` 는 믿을 수 없다 — 실패 후 같은 writer 로 재시도하면 이후 add 된
/// 문서가 조용히 사라지는 것을 관찰했다(num_docs 감소). 그래서 인덱스를 닫고(잠금 해제) 다시 열어
/// 마지막 commit 상태에서 재개한다. P2 색인 워커가 해야 할 복구 절차의 모사다.
fn reopen(slot: &mut Option<SearchIndex>, dir: &Path, opts: &IndexOptions) {
    drop(slot.take());
    *slot = Some(SearchIndex::open_or_create(dir, opts.clone()).expect("reopen index"));
}

#[allow(clippy::too_many_arguments)]
fn single_updates(
    conn: &Connection,
    slot: &mut Option<SearchIndex>,
    dir: &Path,
    opts: &IndexOptions,
    strategy: FilenameStrategy,
    size: u64,
    n: usize,
    rng: &mut Rng,
) -> UpdateStats {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let (mut sq, mut up, mut co, mut se, mut to) = (vec![], vec![], vec![], vec![], vec![]);
    let (mut misses, mut commit_errors, mut stale) = (0, 0, 0);
    let docs_before = slot.as_ref().map_or(0, SearchIndex::num_docs);
    for k in 0..n {
        let id = rng.range(1, size);
        let tag = format!("uptag{}", base36(nonce + k as u128));
        let t = Instant::now();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute(
            "INSERT INTO tags (name, name_key) VALUES (?1, ?1)",
            params![tag],
        )
        .unwrap();
        let tag_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, ?2, 0)",
            params![id as i64, tag_id],
        )
        .unwrap();
        tx.execute(
            "UPDATE media_assets SET revision = revision + 1, updated_at = updated_at + 1 WHERE asset_id = ?1",
            params![id as i64],
        )
        .unwrap();
        tx.commit().unwrap();
        sq.push(t.elapsed());
        let doc = load::load_one(conn, id).expect("load doc");

        let t0 = Instant::now();
        let mut upsert_time = Duration::ZERO;
        let mut commit_time = Duration::ZERO;
        let mut attempt = 0;
        loop {
            let index = slot.as_ref().unwrap();
            let t = Instant::now();
            let outcome = index.upsert(&doc).expect("upsert");
            if outcome != UpsertOutcome::Applied {
                eprintln!("upsert outcome for asset {id}: {outcome:?}");
                stale += 1;
            }
            upsert_time += t.elapsed();
            let t = Instant::now();
            let res = index.commit();
            commit_time += t.elapsed();
            match res {
                Ok(()) => break,
                Err(e) => {
                    // Windows 에서 새 세그먼트 파일 생성이 간헐적으로 PermissionDenied(os error 5)로
                    // 실패한다(원인 미확정: 백신/인덱서 추정). 복구 시간도 지연에 포함한다.
                    attempt += 1;
                    commit_errors += 1;
                    eprintln!("commit failed (update #{k}, attempt {attempt}): {e}");
                    assert!(attempt < 5, "commit kept failing: {e}");
                    let t = Instant::now();
                    reopen(slot, dir, opts);
                    commit_time += t.elapsed();
                }
            }
        }
        let index = slot.as_ref().unwrap();
        let t2 = Instant::now();
        let hits = index.search(&req(&tag, strategy)).expect("search");
        let search_time = t2.elapsed();
        if !hits.iter().any(|h| h.asset_id == id) {
            eprintln!("miss: tag {tag} asset {id} hits={}", hits.len());
            misses += 1;
        }
        up.push(upsert_time);
        co.push(commit_time);
        se.push(search_time);
        to.push(t0.elapsed());
    }
    let docs_after = slot.as_ref().map_or(0, SearchIndex::num_docs);
    UpdateStats {
        n,
        misses,
        commit_errors,
        stale,
        docs_before,
        docs_after,
        sqlite: summarize(&sq),
        upsert: summarize(&up),
        commit: summarize(&co),
        search: summarize(&se),
        total: summarize(&to),
    }
}
