//! 폴더 제거 측정 (T-rmv-bench-01, `#[ignore]` — 수동 실행: `cargo test --release -p mam-core --test folder_removal_bench -- --ignored --nocapture`).
//!
//! 합성 메타데이터만 측정한다 (실제 파일시스템·코덱·Tantivy·UI 성능이 아니다): 자산 N 개(기본 100,000) + 연결 데이터 전 테이블을
//! SQL 로 심고, 제거 소요 시간과 그 동안 읽기 질의(갤러리 첫 페이지 형태) 지연 P50/P95/P99 를 잰다. 데이터는 임시 폴더라 끝나면 지워진다.
//! 환경변수 `MAM_BENCH_N` 으로 개수를 바꾼다.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::env::Env;
use common::purge::{exec, insert_assets};

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn summarize(label: &str, mut ms: Vec<f64>) {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "{label}: n={} P50={:.2}ms P95={:.2}ms P99={:.2}ms max={:.2}ms",
        ms.len(),
        percentile(&ms, 0.50),
        percentile(&ms, 0.95),
        percentile(&ms, 0.99),
        ms.last().copied().unwrap_or(0.0)
    );
}

/// 갤러리 첫 화면과 같은 모양의 읽기: 활성 자산 + 활성 위치 + 태그 존재 확인, 50건
fn first_page(env: &Env, from: i64) -> f64 {
    let t = Instant::now();
    env.read(|c| {
        let mut stmt = c
            .prepare_cached(
                "SELECT a.asset_id, l.rel_path,
                        EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.asset_id)
                 FROM media_assets a JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
                 WHERE a.status = 'active' AND a.asset_id >= ?1
                 ORDER BY a.asset_id LIMIT 50",
            )
            .unwrap();
        let n = stmt.query_map([from], |r| r.get::<_, i64>(0)).unwrap().count();
        std::hint::black_box(n);
    });
    t.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "manual measurement"]
fn t_rmv_bench_01_removal_of_a_large_folder_and_read_latency_during_it() {
    let n: usize = std::env::var("MAM_BENCH_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let env = Arc::new(Env::new());
    std::fs::create_dir_all(env.path("big")).unwrap();
    std::fs::create_dir_all(env.path("keep")).unwrap();
    let big = env
        .ingest
        .add_folder(env.path("big").to_str().unwrap())
        .unwrap()
        .folder_id;
    env.ingest
        .add_folder(env.path("keep").to_str().unwrap())
        .unwrap();

    // 자산 N 개(big) + 남는 폴더 5,000 개(keep). 읽기 질의는 남는 폴더 구간(첫 화면)을 읽는다
    let t = Instant::now();
    let ids = insert_assets(&env, "vol-a", "big", n, "image");
    let keep = insert_assets(&env, "vol-a", "keep", 5_000, "image");
    // 연결 데이터: 모든 big 자산에 전 테이블 한 행 이상 (집합 단위 삽입)
    let (lo, hi) = (ids[0], *ids.last().unwrap());
    exec(
        &env,
        "INSERT OR IGNORE INTO tags (name, name_key) VALUES ('t', 't'); INSERT OR IGNORE INTO collections (name, name_key, created_at) VALUES ('c', 'c', 1);",
    );
    exec(&env, &format!(
        "INSERT INTO asset_tags SELECT asset_id, (SELECT tag_id FROM tags LIMIT 1), 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO collection_assets (collection_id, asset_id, added_at) SELECT (SELECT collection_id FROM collections LIMIT 1), asset_id, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) SELECT asset_id, 'n', 0, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO media_metadata (asset_id, width, height) SELECT asset_id, 1, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) SELECT asset_id, 'thumb', 'done', 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) SELECT asset_id, 'meta', 'done', 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO embeddings (asset_id, vector_kind, source, model_id, model_version, dimension, raw_offset, created_at)
           SELECT asset_id, 'visual', 'image', 'm', '1', 4, 0, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO ai_analyses (asset_id, provider, model, analyzed_at, updated_at) SELECT asset_id, 'p', 'm', 1, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO ai_task_state (asset_id, kind, state, updated_at) SELECT asset_id, 'describe', 'done', 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO analysis_runs (asset_id, kind, provider, ok, attempts, at) SELECT asset_id, 'describe', 'p', 1, 1, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};
         INSERT INTO jobs (kind, asset_id, created_at, updated_at) SELECT 'ai', asset_id, 1, 1 FROM media_assets WHERE asset_id BETWEEN {lo} AND {hi};"
    ));
    exec(&env, "ANALYZE");
    eprintln!(
        "seeded {n} assets + linked rows in {:.1}s",
        t.elapsed().as_secs_f64()
    );

    let from = keep[0];
    // 대조: 제거 없이 읽기만
    let baseline: Vec<f64> = (0..300).map(|_| first_page(&env, from)).collect();
    summarize("read latency, idle", baseline);

    // 제거 + 동시에 읽기
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (env, stop) = (Arc::clone(&env), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut samples = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                samples.push(first_page(&env, from));
                std::thread::sleep(Duration::from_millis(5));
            }
            samples
        })
    };
    let t = Instant::now();
    env.ingest.remove_folder(big).unwrap();
    env.ingest.wait_removals();
    let took = t.elapsed();
    stop.store(true, Ordering::Relaxed);
    let samples = reader.join().unwrap();
    eprintln!(
        "removal of {n} assets: {:.1}s ({:.0} assets/s)",
        took.as_secs_f64(),
        n as f64 / took.as_secs_f64()
    );
    summarize("read latency, during removal", samples);

    assert_eq!(env.count(&format!("SELECT COUNT(*) FROM media_assets WHERE status = 'deleted' AND asset_id BETWEEN {lo} AND {hi}")), n as i64);
    assert_eq!(env.count("SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(
        env.count(&format!(
            "SELECT COUNT(*) FROM media_assets WHERE status = 'active' AND asset_id >= {}",
            keep[0]
        )),
        5_000
    );
}
