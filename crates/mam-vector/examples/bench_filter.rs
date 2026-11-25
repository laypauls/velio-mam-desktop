//! 필터 검색 측정 (D5): F1 검색 중 필터 / F2 후필터 + 후보 확대 / F3 허용 집합 전수 비교 × 선택도.
//!
//! 실행: `bench_filter --dir <dataset> --engine flat --quant f16 --ef 128,256 [--flat-quant f16] [--queries 200]`
//! 전제: ANN 인덱스(`bench_index --phase build`)와 전수 비교 인덱스 `flat-<flat-quant>` 가 먼저 만들어져 있어야 한다.
//!
//! 허용 집합은 **SQLite** 에서 구한다: 온라인 `volume_id` 집합 + `captured_at` 범위 (P1-2 D2 계약).
//! 선택도는 `captured_at` 상한을 분위수로 골라 전체 벡터 수의 s 가 되게 맞춘다. 정답은 허용 벡터 f32 에 대한 전수 비교다.
//! SQL 시간은 세 방식 모두에 필요한 비용이라 따로 보고하고, 방식별 지연에는 넣지 않는다.

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::data::{self, Info, Layout};
use common::{args, stats, sysinfo};
use mam_vector::{
    Engine, FlatIndex, OpenMode, Quantization, SearchParams, exact_topk_f32, open_index,
    search_post_filter,
};

fn main() {
    let a = args::Args::parse();
    let l = Layout::new(
        a.path("dir")
            .unwrap_or_else(|| args::fail("--dir is required")),
    );
    let info = Info::read(&l);
    let spec = common::spec_from_args(&a, info.dim);
    let flat_q =
        Quantization::parse(a.get("flat-quant").unwrap_or("f16")).unwrap_or(Quantization::F16);
    let levels: Vec<f64> = a
        .get("levels")
        .unwrap_or("0.5,0.1,0.01,0.001")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let efs: Vec<usize> = a
        .get("ef")
        .unwrap_or("128")
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let nq: usize = a.num("queries", 200).min(info.queries);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());

    println!(
        "\n== bench_filter {} (flat {}) ==",
        common::index_name(&spec),
        flat_q.as_str()
    );
    let cond = common::print_conditions();
    println!(
        "Dataset: assets={} vectors={} dim={} queries used={nq}",
        info.assets, info.vectors, info.dim
    );

    let ann = open_index(&l.index(&common::index_name(&spec)), &spec, OpenMode::Load)
        .unwrap_or_else(|e| args::fail(&format!("open ann: {e}")));
    let flat = FlatIndex::open(
        &Engine::Flat.data_path(&l.index(&format!("flat-{}", flat_q.as_str()))),
        info.dim,
        flat_q,
        OpenMode::View,
    )
    .unwrap_or_else(|e| args::fail(&format!("open flat: {e}")));
    flat.set_threads(threads);
    let queries = data::read_queries(&l, &info);
    let conn = common::open_db(&l.db());

    // 온라인 볼륨 집합 (D2): availability 대신 volume_id 목록으로 건다
    let online: Vec<i64> = conn
        .prepare("SELECT volume_id FROM volumes WHERE availability = 'online' ORDER BY volume_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let in_list = online
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let base_sql = format!(
        "SELECT e.embedding_id FROM poc_embeddings e
         JOIN file_locations l ON l.asset_id = e.asset_id
         JOIN media_metadata m ON m.asset_id = e.asset_id
         WHERE l.volume_id IN ({in_list}) AND m.captured_at <= ?1"
    );
    // 분위수 보정용: 온라인 + captured_at 있는 벡터의 captured_at 정렬
    let caps: Vec<i64> = conn
        .prepare(&format!(
            "SELECT m.captured_at FROM poc_embeddings e
             JOIN file_locations l ON l.asset_id = e.asset_id
             JOIN media_metadata m ON m.asset_id = e.asset_id
             WHERE l.volume_id IN ({in_list}) AND m.captured_at IS NOT NULL ORDER BY m.captured_at"
        ))
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    println!(
        "online volumes: {online:?}, vectors online with captured_at: {} ({:.1}% of all)",
        caps.len(),
        100.0 * caps.len() as f64 / info.vectors as f64
    );

    let f = std::fs::File::open(l.vectors()).unwrap();
    // SAFETY: 읽기 전용 매핑 (정답 계산용). 측정 중 파일은 바뀌지 않는다.
    let map = unsafe { memmap2::Mmap::map(&f).unwrap() };
    let all_rows: &[f32] = bytemuck::cast_slice(&map[..]);

    let mut json_levels = Vec::new();
    for &s in &levels {
        let want = ((s * info.vectors as f64) as usize).clamp(1, caps.len());
        let cap_hi = caps[want - 1];
        // SQL: 허용 embedding_id (측정)
        let t = Instant::now();
        let mut allowed: Vec<u64> = conn
            .prepare(&base_sql)
            .unwrap()
            .query_map([cap_hi], |r| r.get::<_, i64>(0).map(|x| x as u64))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        allowed.sort_unstable();
        let sql_ms = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let mut bitmap = vec![false; info.vectors + 1];
        allowed.iter().for_each(|&id| bitmap[id as usize] = true);
        let bitmap_ms = t.elapsed().as_secs_f64() * 1000.0;
        let actual = allowed.len() as f64 / info.vectors as f64;

        // 정답: 허용 벡터 f32 를 모아 전수 비교
        let mut gathered = Vec::with_capacity(allowed.len() * info.dim);
        for &id in &allowed {
            let r = (id as usize - 1) * info.dim;
            gathered.extend_from_slice(&all_rows[r..r + info.dim]);
        }
        let truth: Vec<Vec<u64>> = queries
            .iter()
            .take(nq)
            .map(|q| {
                exact_topk_f32(&allowed, &gathered, info.dim, q, 10, threads)
                    .iter()
                    .map(|h| h.id)
                    .collect()
            })
            .collect();
        drop(gathered);

        println!(
            "\n### selectivity target {:.3}% -> actual {:.3}% ({} vectors), captured_at <= {cap_hi}",
            s * 100.0,
            actual * 100.0,
            allowed.len()
        );
        println!("SQL allowed-set query: {sql_ms:.1} ms, bitmap build: {bitmap_ms:.1} ms");
        println!(
            "\n| method | ef | P50 ms | P95 ms | P99 ms | Recall@10 | avg hits | incomplete % | avg rounds / k_final |\n|---|---:|---:|---:|---:|---:|---:|---:|---|"
        );
        let allow = |id: u64| bitmap.get(id as usize).copied().unwrap_or(false);
        let mut rows_json = Vec::new();
        for &ef in &efs {
            let p = SearchParams { ef_search: ef };
            // F1
            rows_json.push(report("F1", ef, nq, &queries, &truth, |q| {
                let h = ann.search(q, 10, &p, Some(&allow)).unwrap();
                (h.iter().map(|x| x.id).collect(), 1.0, 10.0, false)
            }));
            // F2 (후보 4배 -> 부족하면 4배씩 확대)
            rows_json.push(report("F2", ef, nq, &queries, &truth, |q| {
                let r = search_post_filter(&*ann, q, 10, &p, &allow, 4).unwrap();
                (
                    r.hits.iter().map(|x| x.id).collect(),
                    r.rounds as f64,
                    r.k_final as f64,
                    r.incomplete,
                )
            }));
        }
        // F3: 허용 집합만 전수 비교 (ef 무관)
        rows_json.push(report("F3", 0, nq, &queries, &truth, |q| {
            let h = flat.search_subset(q, 10, &allowed).unwrap();
            (
                h.iter().map(|x| x.id).collect(),
                1.0,
                allowed.len() as f64,
                false,
            )
        }));
        json_levels.push(format!(
            "{{\"target\":{s},\"actual\":{actual:.5},\"allowed\":{},\"sql_ms\":{sql_ms:.1},\"bitmap_ms\":{bitmap_ms:.1},\"rows\":[{}]}}",
            allowed.len(), rows_json.join(",")
        ));
    }
    println!(
        "\npeak working set: {:.1} MB",
        sysinfo::mb(sysinfo::peak_rss_bytes())
    );
    println!(
        "JSON: {{\"tool\":\"bench_filter\",\"name\":{},\"vectors\":{},\"levels\":[{}],\"conditions\":{}}}",
        common::json_str(&common::index_name(&spec)),
        info.vectors,
        json_levels.join(","),
        common::conditions_json(&cond)
    );
}

/// 질의 `nq` 개를 돌려 지연·Recall 을 한 줄로 출력하고 JSON 조각을 돌려준다.
/// 클로저 반환: (결과 id, 라운드 수, 최종 k, 불완전 여부).
fn report(
    name: &str,
    ef: usize,
    nq: usize,
    queries: &[Vec<f32>],
    truth: &[Vec<u64>],
    mut run: impl FnMut(&[f32]) -> (Vec<u64>, f64, f64, bool),
) -> String {
    let (mut lat, mut rec, mut hits, mut rounds, mut kf, mut inc) =
        (Vec::<Duration>::new(), 0.0, 0.0, 0.0, 0.0, 0usize);
    for (q, t) in queries.iter().take(nq).zip(truth) {
        let s = Instant::now();
        let (ids, r, k, incomplete) = run(q);
        lat.push(s.elapsed());
        let want = t.len().max(1);
        rec += ids.iter().filter(|i| t.contains(i)).count() as f64 / want as f64;
        hits += ids.len() as f64;
        rounds += r;
        kf += k;
        inc += incomplete as usize;
    }
    let st = stats::summarize(&lat);
    let n = nq as f64;
    println!(
        "| {name} | {ef} | {:.2} | {:.2} | {:.2} | {:.4} | {:.1} | {:.1} | {:.2} / {:.0} |",
        st.p50_ms,
        st.p95_ms,
        st.p99_ms,
        rec / n,
        hits / n,
        100.0 * inc as f64 / n,
        rounds / n,
        kf / n
    );
    format!(
        "{{\"method\":\"{name}\",\"ef\":{ef},\"lat\":{},\"recall10\":{:.4},\"avg_hits\":{:.2},\"incomplete_pct\":{:.1},\"avg_rounds\":{:.2},\"avg_k_final\":{:.0}}}",
        st.json(),
        rec / n,
        hits / n,
        100.0 * inc as f64 / n,
        rounds / n,
        kf / n
    )
}
