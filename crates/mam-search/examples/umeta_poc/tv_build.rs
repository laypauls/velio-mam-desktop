//! 100만 건 Tantivy 색인 구축 + 크기 측정.

use std::path::Path;
use std::time::Instant;

use tantivy::Index;

use crate::data::{gen_values, row_count, specs};
use crate::q::{build_schema, make_doc};

pub fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// `with_sortable`: number 필드의 정렬 가능 i64 경로(`n{id}`)도 함께 색인한다 (대안 비교용).
pub fn build(
    dir: &Path,
    n: u64,
    threads: usize,
    heap_mb: usize,
    with_sortable: bool,
    with_umeta: bool,
) {
    mam_search::korean::ensure_ready().expect("ko dict");
    if dir.exists() {
        std::fs::remove_dir_all(dir).unwrap();
    }
    std::fs::create_dir_all(dir).unwrap();
    let (schema, fx) = build_schema();
    let index = Index::create_in_dir(dir, schema).unwrap();
    mam_search::analyzer::register(&index);
    let mut w = index
        .writer_with_num_threads(threads, heap_mb * 1_000_000)
        .unwrap();
    let sp = specs();
    let t0 = Instant::now();
    let mut gen_ns = 0u128;
    let mut rows = 0u64;
    for id in 1..=n {
        let g = Instant::now();
        let vals = if with_umeta {
            gen_values(id, &sp, 0)
        } else {
            Vec::new()
        };
        let doc = make_doc(&fx, id, &vals, with_sortable);
        gen_ns += g.elapsed().as_nanos();
        rows += row_count(&vals) as u64;
        w.add_document(doc).unwrap();
        if id % 200_000 == 0 {
            println!(
                "  indexed {id} docs  elapsed {:.1}s",
                t0.elapsed().as_secs_f64()
            );
        }
    }
    let add_done = t0.elapsed();
    let c = Instant::now();
    w.commit().unwrap();
    let commit = c.elapsed();
    let m = Instant::now();
    w.wait_merging_threads().unwrap();
    let merge = m.elapsed();
    let total = t0.elapsed();
    let segs = Index::open_in_dir(dir)
        .unwrap()
        .searchable_segment_metas()
        .unwrap()
        .len();
    println!(
        "BUILD n={n} writer_threads={threads} heap={heap_mb}MB sortable_paths={with_sortable}\n  \
         add_documents={:.1}s (doc build inside loop {:.1}s) commit={:.1}s merge_wait={:.1}s total={:.1}s\n  \
         value_rows(sqlite equiv)={rows} segments={segs} index_size={:.1} MB",
        add_done.as_secs_f64(),
        gen_ns as f64 / 1e9,
        commit.as_secs_f64(),
        merge.as_secs_f64(),
        total.as_secs_f64(),
        dir_size(dir) as f64 / 1_048_576.0,
    );
    // 필드별 크기 (색인 파일 확장자 기준)
    let mut by_ext: std::collections::BTreeMap<String, u64> = Default::default();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let ext = e
            .path()
            .extension()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        *by_ext.entry(ext).or_default() += e.metadata().map(|m| m.len()).unwrap_or(0);
    }
    for (k, v) in by_ext {
        println!("    .{k}: {:.1} MB", v as f64 / 1_048_576.0);
    }
}
