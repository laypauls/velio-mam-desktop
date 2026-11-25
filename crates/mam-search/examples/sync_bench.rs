//! Outbox 색인 워커 측정 (P2-4) — 제품 `SyncWorker` 로 100만 건을 다룰 때의 비용.
//!
//! 실행: `cargo run --release -p mam-search --example sync_bench -- --size 1000000`
//! 옵션: `--out <dir>`(데이터셋 폴더, 기본 P1-2 배치), `--edits 200`(사용자 편집 횟수),
//! `--bulk 20000`(저우선 이벤트 수), `--seed 7`
//!
//! 단계: ① 빈 디렉터리에서 `SyncWorker::open` -> 전체 재구축(SQLite 100만 건 -> 인덱스) 시간 ② 재시작
//! (기존 인덱스 열기 + 문서 수 확인) 시간 ③ 사용자 편집 1건씩 `process_batch` 지연 + 검색 가능 확인
//! ④ 대량 이벤트 배치 처리량. 인덱스는 `<out>/index-sync` 에 만들고, 데이터셋 DB 에 태그/revision
//! 변경과 outbox 행을 **실제로 기록**한다 (다시 측정하려면 `gen_dataset` 부터).
//! 재구축 문서에는 AI 설명·메모가 없다(제품 질의는 V1 테이블만 읽는다) — P1-2 `build_index` 와 문서 구성이 다르다.

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::rng::Rng;
use common::stats::summarize;
use common::{args, json_str, sysinfo};
use mam_db::SystemClock;
use mam_search::sync::{SyncConfig, SyncWorker};
use mam_search::{FilenameStrategy, IndexOptions, SearchRequest};
use rusqlite::{Connection, params};

/// 태그 1개를 붙이고 revision 을 올리며 outbox 이벤트를 같은 트랜잭션에 기록한다.
fn edit(conn: &Connection, asset_id: u64, tag: &str, priority: i64) {
    let tx = conn.unchecked_transaction().expect("tx");
    tx.execute(
        "INSERT OR IGNORE INTO tags (name, name_key) VALUES (?1, lower(?1))",
        [tag],
    )
    .expect("tag");
    tx.execute(
        "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id, created_at)
         SELECT ?1, tag_id, 0 FROM tags WHERE name_key = lower(?2)",
        params![asset_id as i64, tag],
    )
    .expect("link");
    tx.execute(
        "UPDATE media_assets SET revision = revision + 1 WHERE asset_id = ?1",
        [asset_id as i64],
    )
    .expect("bump");
    tx.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         SELECT asset_id, revision, 'upsert', ?2, 0 FROM media_assets WHERE asset_id = ?1",
        params![asset_id as i64, priority],
    )
    .expect("event");
    tx.commit().expect("commit");
}

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let edits: usize = a.num("edits", 200);
    let bulk: usize = a.num("bulk", 20_000);
    let mut rng = Rng::new(a.num("seed", 7));
    let layout = common::Layout::new(size, a.path("out"));
    let dir = layout.dir.join("index-sync");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("remove old index");
    }
    let conn = common::open_db(&layout.db());
    let cond = sysinfo::conditions();

    // ① 전체 재구축
    let t = Instant::now();
    let (worker, outcome) = SyncWorker::open(
        &conn,
        &dir,
        IndexOptions::strategy_b(),
        SyncConfig::default(),
    )
    .expect("open + rebuild");
    let rebuild = t.elapsed();
    let report = outcome.rebuilt.expect("empty dir must rebuild");
    let rebuild_rss = sysinfo::peak_rss_bytes();
    let index_bytes = common::dir_size(&dir);
    println!(
        "rebuild: reason={:?} docs={} cleanup_error={:?}",
        report.reason, report.docs, report.cleanup_error
    );
    let segments = common::segment_count(&dir);
    drop(worker);

    // ② 재시작 (기존 인덱스 열기 + 문서 수 확인)
    let t = Instant::now();
    let (worker, outcome) = SyncWorker::open(
        &conn,
        &dir,
        IndexOptions::strategy_b(),
        SyncConfig::default(),
    )
    .expect("reopen");
    let reopen = t.elapsed();
    assert!(outcome.rebuilt.is_none(), "reopen must not rebuild");

    // ③ 사용자 편집 1건 -> process_batch 지연, 검색 가능 확인
    let (mut apply, mut visible) = (Vec::new(), Vec::new());
    let mut missing = 0;
    for i in 0..edits {
        let asset = rng.range(1, size);
        let tag = format!("zbench{i}x");
        edit(&conn, asset, &tag, 0);
        let t = Instant::now();
        let r = worker.process_batch(&conn, &SystemClock).expect("batch");
        apply.push(t.elapsed());
        assert_eq!(r.events_done, 1, "{r:?}");
        let t = Instant::now();
        let hits = worker
            .index()
            .expect("index")
            .search(&SearchRequest {
                text: tag,
                strategy: FilenameStrategy::B,
                ..Default::default()
            })
            .expect("search");
        visible.push(t.elapsed());
        if !hits.iter().any(|h| h.asset_id == asset) {
            missing += 1;
        }
    }

    // ④ 대량 이벤트 (priority 1) — 지연 없이 배치 처리량
    let cfg = worker.config().clone();
    for i in 0..bulk {
        edit(&conn, rng.range(1, size), &format!("zbulk{}", i % 50), 1);
    }
    let t = Instant::now();
    let mut batch_times = Vec::new();
    let mut done = 0;
    loop {
        let tb = Instant::now();
        let r = worker
            .process_batch(&conn, &SystemClock)
            .expect("bulk batch");
        if r.events_selected == 0 {
            break;
        }
        assert!(r.error.is_none(), "{r:?}");
        done += r.events_done;
        batch_times.push(tb.elapsed());
    }
    let bulk_total = t.elapsed();
    let docs_after = worker.index().expect("index").num_docs();

    let s = |d: Duration| d.as_secs_f64();
    let apply_s = summarize(&apply);
    let visible_s = summarize(&visible);
    let batch_s = summarize(&batch_times);
    println!("\n== sync_bench ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: size={size} strategy=B index_docs_after={docs_after} segments_after_rebuild={segments}"
    );
    println!("| step | value |\n|---|---:|");
    println!(
        "| full rebuild from SQLite (open->swap, incl. merge wait) | {:.1} s |",
        s(rebuild)
    );
    println!(
        "| rebuild docs/sec | {:.0} |",
        report.docs as f64 / s(rebuild)
    );
    println!("| index dir size | {:.1} MB |", sysinfo::mb(index_bytes));
    println!(
        "| peak RSS (process, through rebuild) | {:.0} MB |",
        sysinfo::mb(rebuild_rss)
    );
    println!(
        "| restart (open existing + count check) | {:.2} s |",
        s(reopen)
    );
    println!(
        "| user edit process_batch P50/P95/P99 ({} edits) | {:.1} / {:.1} / {:.1} ms |",
        apply_s.n, apply_s.p50_ms, apply_s.p95_ms, apply_s.p99_ms
    );
    println!(
        "| search after edit P50/P95/P99 | {:.2} / {:.2} / {:.2} ms |",
        visible_s.p50_ms, visible_s.p95_ms, visible_s.p99_ms
    );
    println!("| edits not found by search | {missing} |");
    println!(
        "| bulk: {done} events, batch_size={} | {:.1} s ({:.0} events/s) |",
        cfg.batch_size,
        s(bulk_total),
        done as f64 / s(bulk_total)
    );
    println!(
        "| bulk batch time P50/P95/P99 | {:.0} / {:.0} / {:.0} ms |",
        batch_s.p50_ms, batch_s.p95_ms, batch_s.p99_ms
    );
    println!(
        "JSON: {{\"tool\":\"sync_bench\",\"size\":{size},\"rebuild_s\":{:.2},\"rebuild_docs\":{},\"index_bytes\":{index_bytes},\"peak_rss_bytes\":{rebuild_rss},\"reopen_s\":{:.3},\"edit_apply\":{},\"edit_search\":{},\"edits_missing\":{missing},\"bulk_events\":{done},\"bulk_s\":{:.2},\"bulk_batch\":{},\"conditions\":{},\"index_dir\":{}}}",
        s(rebuild),
        report.docs,
        s(reopen),
        apply_s.json(),
        visible_s.json(),
        s(bulk_total),
        batch_s.json(),
        common::conditions_json(&cond),
        json_str(&dir.display().to_string())
    );
}
