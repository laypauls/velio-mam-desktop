//! SQLite 전체에서 Tantivy 인덱스를 재구축한다 — "SQLite 에서 전체 재구축" 시간 측정.
//!
//! 실행: `cargo run --release -p mam-search --example build_index -- --size 10000 --strategy a`
//! 옵션: `--out <dir>`, `--threads N`(기본 전략 옵션값), `--heap-mb N`, `--quiet-secs 3`(병합 종료 판정 구간)
//!
//! 인덱스 디렉터리(`index-<전략>`)가 있으면 지우고 새로 만든다 (전략마다 별도 디렉터리).

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::{args, json_str, load, sysinfo};
use mam_search::SearchIndex;

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let (letter, _strategy, mut opts) = common::parse_strategy(a.get("strategy").unwrap_or("a"));
    if let Some(t) = a.get("threads") {
        opts.writer_threads = t.parse().unwrap_or_else(|_| args::fail("--threads"));
    }
    if let Some(h) = a.get("heap-mb") {
        let mb: usize = h.parse().unwrap_or_else(|_| args::fail("--heap-mb"));
        opts.writer_heap_bytes = mb * 1_000_000;
    }
    let quiet = Duration::from_secs(a.num("quiet-secs", 3));
    let layout = common::Layout::new(size, a.path("out"));
    let index_dir = layout.index(letter);
    if index_dir.exists() {
        std::fs::remove_dir_all(&index_dir).expect("remove old index");
    }

    let conn = common::open_db(&layout.db());
    let seed = common::setting(&conn, "poc.seed").unwrap_or_default();
    let (threads, heap) = (opts.writer_threads, opts.writer_heap_bytes);

    let t_total = Instant::now();
    let index = SearchIndex::open_or_create(&index_dir, opts).expect("open index");
    let mut stmt = conn.prepare(load::ALL_DOCS_SQL).expect("prepare");
    let mut rows = stmt.query([]).expect("query");

    let (mut read, mut indexing) = (Duration::ZERO, Duration::ZERO);
    let mut docs = 0u64;
    loop {
        let t = Instant::now();
        let doc = match rows.next().expect("row") {
            Some(r) => load::doc_from_row(r).expect("map row"),
            None => break,
        };
        read += t.elapsed();
        let t = Instant::now();
        index.upsert_unguarded(&doc).expect("upsert");
        indexing += t.elapsed();
        docs += 1;
    }
    let t = Instant::now();
    index.commit().expect("commit");
    let commit = t.elapsed();
    let segs_after_commit = common::segment_count(&index_dir);
    let (merge_wait, timed_out) =
        common::wait_for_merges(&index_dir, quiet, Duration::from_secs(1800));
    let total = t_total.elapsed();
    let segs_after_merge = common::segment_count(&index_dir);
    let bytes = common::dir_size(&index_dir);
    let peak = sysinfo::peak_rss_bytes();
    let live = index.num_docs();
    drop(index);

    let cond = sysinfo::conditions();
    let s = |d: Duration| d.as_secs_f64();
    println!("\n== build_index (full rebuild from SQLite) ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: size={size} dataset_seed={seed} strategy={} docs_indexed={docs} live_docs={live}",
        letter.to_ascii_uppercase()
    );
    println!(
        "Writer: threads={threads} heap={}MB (total across threads)",
        heap / 1_000_000
    );
    println!(
        "Index state: segments after commit={segs_after_commit}, after merge wait={segs_after_merge}{}",
        if timed_out {
            " (MERGE WAIT TIMED OUT)"
        } else {
            ""
        }
    );
    println!("| step | seconds |\n|---|---:|");
    println!("| SQLite read (fetch+map, interleaved) | {:.2} |", s(read));
    println!("| indexing (upsert_unguarded calls) | {:.2} |", s(indexing));
    println!("| commit | {:.2} |", s(commit));
    println!(
        "| (note) real tokenizing/segment work runs in writer threads: indexing+commit | {:.2} |",
        s(indexing + commit)
    );
    println!(
        "| merge wait (to last change; heuristic, quiet={}s) | {:.2} |",
        quiet.as_secs(),
        s(merge_wait)
    );
    println!(
        "| total wall (open..merge settled, incl. quiet window) | {:.2} |",
        s(total)
    );
    println!("\n| metric | value |\n|---|---:|");
    println!(
        "| docs/sec (docs / (total - quiet window)) | {:.0} |",
        docs as f64 / (s(total) - quiet.as_secs_f64()).max(0.001)
    );
    println!("| index dir size | {:.1} MB |", sysinfo::mb(bytes));
    println!(
        "| bytes per doc | {:.0} |",
        bytes as f64 / docs.max(1) as f64
    );
    println!("| peak RSS (working set) | {:.0} MB |", sysinfo::mb(peak));
    println!(
        "JSON: {{\"tool\":\"build_index\",\"size\":{size},\"strategy\":\"{}\",\"docs\":{docs},\"threads\":{threads},\"heap_bytes\":{heap},\"read_s\":{:.2},\"index_s\":{:.2},\"commit_s\":{:.2},\"merge_wait_s\":{:.2},\"total_s\":{:.2},\"segments_after_commit\":{segs_after_commit},\"segments_after_merge\":{segs_after_merge},\"merge_timed_out\":{timed_out},\"index_bytes\":{bytes},\"peak_rss_bytes\":{peak},\"conditions\":{},\"index_dir\":{}}}",
        letter,
        s(read),
        s(indexing),
        s(commit),
        s(merge_wait),
        s(total),
        common::conditions_json(&cond),
        json_str(&index_dir.display().to_string())
    );
}
