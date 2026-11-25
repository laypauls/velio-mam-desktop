//! 벡터 성능 수정 전/후 측정 — 활성 맵 캐시, `EmbedStore::put` 상수화, `find_similar` 단일 순회, 필터 전략 임계.
//!
//! 데이터는 `bench_scale --phase build --scale-div N` 이 만든 디렉터리를 쓴다 (그 위에 put 을 몇 건 더 쓴다 — 측정용 복사본에서만).
//! 실행: `cargo run --release -p mam-vector --example bench_perf_fix -- --dir D:\mam-bench\perf-fix\data --scale-div 5`
#![allow(dead_code)]

#[path = "common/mod.rs"]
mod common;
#[path = "common/legacy_similar.rs"]
mod legacy_similar;
#[path = "common/scale_build.rs"]
mod scale_build;
#[path = "common/scale_data.rs"]
mod scale_data;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use common::stats::{self, Summary};
use common::{args, open_db, print_conditions};
use legacy_similar::legacy_find_similar;
use mam_vector::{
    ActiveMapCache, AnnIndex, EmbedStore, Generations, IdSet, NewVector, OpenMode, SearchParams,
    SimilarQuery, VectorSource, active_candidates, active_candidates_cached, find_similar,
    plan_candidates,
};
use scale_build::{Paths, visual_key};
use scale_data::{Mixture, Scale, allowed_eids, item_queries, video_queries};

fn time<T>(mut f: impl FnMut() -> T, runs: usize) -> Summary {
    let samples: Vec<Duration> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            std::hint::black_box(f());
            t.elapsed()
        })
        .collect();
    stats::summarize(&samples)
}

fn row(label: &str, s: &Summary) {
    println!(
        "| {label} | {} | {:.2} | {:.2} | {:.2} | {:.2} |",
        s.n, s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
}

fn header(title: &str) {
    println!(
        "\n#### {title}\n| 항목 | n | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|"
    );
}

fn main() {
    let a = args::Args::parse();
    let dir = a
        .path("dir")
        .unwrap_or_else(|| args::fail("--dir required"));
    let sc = Scale::new(a.num("scale-div", 5u64));
    let n = a.num("queries", 100usize);
    let p = Paths { root: dir };
    println!("== bench_perf_fix ==");
    print_conditions();
    println!(
        "dataset: 자산 {}, 시각 {} x 512-d, 의미 {} x 384-d, 스레드 {}",
        sc.assets(),
        sc.visual(),
        sc.assets(),
        std::thread::available_parallelism().map_or(4, |n| n.get())
    );
    let mut conn = open_db(&p.db());
    let store = EmbedStore::new(p.vectors()).unwrap();
    let vk = visual_key();
    let g = Generations::new(&store, &vk).unwrap();
    let vis = g.open_active(&vk, OpenMode::View).unwrap().expect("index");
    let total = vis.len();
    let plan = plan_candidates(50);
    let vq = item_queries(&Mixture::visual(), n + 5);

    // ---------------------------------------------------------------- 1. 활성 맵
    header("1. 활성 맵 (시각)");
    let legacy = time(|| store.asset_map(&conn, &vk).unwrap(), 6);
    row("이전: 질의마다 `asset_map` 전체 로드 (HashMap)", &legacy);
    let cache = ActiveMapCache::new();
    let t = Instant::now();
    let map = cache.get_or_load(&conn, &store, &vk).unwrap();
    let cold = t.elapsed();
    println!(
        "| 이후: 캐시 첫 로드 (1회) | 1 | {:.0} | | | |",
        cold.as_secs_f64() * 1e3
    );
    drop(map);
    row(
        "이후: 캐시 적중 `get_or_load` (변경 없음)",
        &time(|| cache.get_or_load(&conn, &store, &vk).unwrap(), 2000),
    );
    let map = cache.get_or_load(&conn, &store, &vk).unwrap();
    println!(
        "\n맵 {} 행, 압축 표현 {:.1} MB (HashMap 추정 {:.1} MB)",
        map.len(),
        map.len() as f64 * 16.0 / 1048576.0,
        (map.len() * 8 / 7).next_power_of_two() as f64 * 17.0 / 1048576.0
    );
    drop(map);

    // ---------------------------------------------------------------- 2. put (+ 증분 갱신)
    header("2. `EmbedStore::put` 1건 (fsync 포함) + 증분 갱신");
    let old_floor = time(
        || {
            conn.query_row(
                "SELECT MAX(raw_offset) FROM embeddings WHERE vector_kind='visual' AND model_id=?1 AND model_version=?2 AND dimension=?3",
                rusqlite::params![vk.model_id, vk.model_version, vk.dimension as i64],
                |r| r.get::<_, Option<i64>>(0),
            )
            .unwrap()
        },
        5,
    );
    row("이전 put 이 매번 더한 `MAX(raw_offset)` 쿼리", &old_floor);
    let t = Instant::now();
    store.put(&mut conn, &vk, 1, &[nv(&vq[0])], 1).unwrap();
    println!(
        "| 이후 put 첫 호출 (키당 1회 floor 조회 포함) | 1 | {:.0} | | | |",
        t.elapsed().as_secs_f64() * 1e3
    );
    let mut puts = Vec::new();
    let mut refresh = Vec::new();
    for i in 0..20usize {
        let asset = 1 + (i as i64 % 7); // 같은 자산 재임베딩 = 폐기 + 새 행
        let t = Instant::now();
        store
            .put(&mut conn, &vk, asset, &[nv(&vq[i + 1])], 2 + i as i64)
            .unwrap();
        puts.push(t.elapsed());
        let t = Instant::now();
        cache.get_or_load(&conn, &store, &vk).unwrap();
        refresh.push(t.elapsed());
    }
    row("이후 put (이후 호출, 재임베딩)", &stats::summarize(&puts));
    row(
        "이후 put 직후 `get_or_load` 증분 갱신 (폐기 1 + 추가 1 + 검증)",
        &stats::summarize(&refresh),
    );
    let fresh = store.asset_map(&conn, &vk).unwrap();
    let cached = cache.get_or_load(&conn, &store, &vk).unwrap();
    assert_eq!(fresh.len(), cached.len(), "cache == SQLite after puts");
    assert!(fresh.iter().all(|(e, a)| cached.asset_of(*e) == Some(*a)));
    println!("(검증: put 20건 뒤 캐시 == SQLite 활성 행)");
    {
        let edir = p.root.join("empty-put");
        let _ = std::fs::remove_dir_all(&edir);
        std::fs::create_dir_all(&edir).unwrap();
        let mut ec = open_db(&edir.join("e.db"));
        mam_db::migrate(&mut ec, &edir.join("backup"), &mam_db::SystemClock).unwrap();
        for a in 1..=7 {
            ec.execute(
                "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at) VALUES (?1, 'image', 1, 1, 1, 1)",
                [a],
            )
            .unwrap();
        }
        let es = EmbedStore::new(edir.join("vectors")).unwrap();
        es.put(&mut ec, &vk, 1, &[nv(&vq[0])], 1).unwrap();
        let ts: Vec<Duration> = (0..20usize)
            .map(|i| {
                let t = Instant::now();
                es.put(
                    &mut ec,
                    &vk,
                    1 + (i as i64 % 7),
                    &[nv(&vq[i + 1])],
                    2 + i as i64,
                )
                .unwrap();
                t.elapsed()
            })
            .collect();
        row(
            "참고: 빈 DB 의 put (fsync·트랜잭션 하한)",
            &stats::summarize(&ts),
        );
        drop(ec);
        let _ = std::fs::remove_dir_all(&edir);
    }

    // ---------------------------------------------------------------- 3. 앱 경로
    header("3. 하이브리드 벡터 후보 e2e (맵 준비 포함, 12 스레드)");
    let ids10 = allowed_eids(&sc, 10);
    let qs = &vq[..8];
    row(
        "이전 `active_candidates` 무필터",
        &time_q(qs, |q| {
            active_candidates(&conn, &store, &vk, &vis, q, &plan, None).unwrap()
        }),
    );
    row(
        "이전 `active_candidates` 허용 10%",
        &time_q(qs, |q| {
            active_candidates(&conn, &store, &vk, &vis, q, &plan, Some(&ids10)).unwrap()
        }),
    );
    let qs = &vq[..n];
    row(
        "이후 `active_candidates_cached` 무필터",
        &time_q(qs, |q| {
            active_candidates_cached(&conn, &store, &cache, &vk, &vis, q, &plan, None).unwrap()
        }),
    );
    row(
        "이후 `active_candidates_cached` 허용 10%",
        &time_q(qs, |q| {
            active_candidates_cached(&conn, &store, &cache, &vk, &vis, q, &plan, Some(&ids10))
                .unwrap()
        }),
    );

    // ---------------------------------------------------------------- 4. find_similar
    header("4. 유사 미디어 (질의 프레임 5개, 50건)");
    let hm: HashMap<u64, u64> = store.asset_map(&conn, &vk).unwrap();
    let hm_of = |e: u64| hm.get(&e).copied();
    let am = cache.get_or_load(&conn, &store, &vk).unwrap();
    let am_of = |e: u64| am.asset_of(e);
    let vqs = video_queries(&Mixture::visual(), &sc, n + 5);
    let run_old = |(a, f): &(u64, Vec<Vec<f32>>)| {
        legacy_find_similar(&vis, f, std::slice::from_ref(a), 50, &hm_of).unwrap()
    };
    let run_new_am = |(a, f): &(u64, Vec<Vec<f32>>)| {
        let q = SimilarQuery {
            frames: f,
            exclude: std::slice::from_ref(a),
            limit: 50,
            cursor: None,
            min_score: 0.0,
        };
        find_similar(&vis, &q, &am_of).unwrap()
    };
    let mut same = 0;
    for q in &vqs[..10] {
        let (o, nw) = (run_old(q), run_new_am(q).items);
        let new_by: HashMap<u64, f32> = nw.iter().copied().collect();
        same += usize::from(
            o.len() == nw.len()
                && o.iter()
                    .all(|(a, s)| new_by.get(a).is_some_and(|t| (s - t).abs() < 1e-5)),
        );
    }
    row(
        "이전 (프레임별 스캔 + 벡터마다 해시 조회)",
        &time_q(&vqs[..n], run_old),
    );
    row(
        "이후 (단일 순회, 스캔 뒤 걸러냄), 맵=ActiveMap",
        &time_q(&vqs[..n], run_new_am),
    );
    println!("(동등성: 처음 10개 질의 중 {same}개가 이전 결과와 같은 자산 집합·유사도(차 < 1e-5))");

    // ---------------------------------------------------------------- 5. 필터 전략 임계
    header("5. 필터 전략 강제 비교 (임베딩 단위 k=400, 자산 묶기 제외, 허용 집합 구성 포함)");
    let params = SearchParams::default();
    for pct in [1u64, 2, 3, 5, 10, 33, 50, 70] {
        let ids = allowed_eids(&sc, pct);
        let sub = time_q(&vq[..n], |q| vis.search_subset(q, 400, &ids).unwrap());
        let full = time_q(&vq[..n], |q| {
            let set = IdSet::from_ids(&ids);
            let pred = |id: u64| set.contains(id);
            vis.search(q, 400, &params, Some(&pred)).unwrap()
        });
        let ratio = ids.len() as f64 / total as f64;
        row(
            &format!("허용 {pct}% ({:.1}% 벡터) Subset", ratio * 100.0),
            &sub,
        );
        row(
            &format!("허용 {pct}% ({:.1}% 벡터) FullScan(비트맵)", ratio * 100.0),
            &full,
        );
    }
}

/// 질의 입력별 시간 (워밍업 2건 제외).
fn time_q<Q, T>(qs: &[Q], mut f: impl FnMut(&Q) -> T) -> Summary {
    qs.iter().take(2).for_each(|q| {
        std::hint::black_box(f(q));
    });
    let samples: Vec<Duration> = qs
        .iter()
        .skip(2)
        .map(|q| {
            let t = Instant::now();
            std::hint::black_box(f(q));
            t.elapsed()
        })
        .collect();
    stats::summarize(&samples)
}

fn nv(v: &[f32]) -> NewVector<'_> {
    NewVector {
        segment_id: None,
        source: VectorSource::Image,
        frame_index: None,
        vector: v,
    }
}
