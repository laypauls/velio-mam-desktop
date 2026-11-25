//! P4 규모 측정 — Recall@10: 같은 합성 벡터를 f32 로 다시 만들어 `exact_topk_f32` 전수 비교 정답과 대조한다.
//! 정답은 f32 원본, 대상은 제품 경로(f16 flat 세대를 view 로 열어 `vector_candidates`·`find_similar`).
//! 자산 id 는 적재 배치 산술로 구한다 (SQLite 맵과의 일치는 latency 단계가 검증).

use std::collections::HashSet;

use mam_vector::{
    AnnIndex, EmbedStore, FlatIndex, Generations, Hit, OpenMode, SearchParams, SimilarQuery,
    exact_topk_f32, find_similar, group_by_asset, plan_candidates, vector_candidates,
};

use crate::scale_build::{Paths, semantic_key, visual_key};
use crate::scale_data::{
    Mixture, SEM_DIM, Scale, VIS_DIM, allowed_eids, fill_semantic, fill_visual, item_queries,
    video_queries,
};

struct Acc(Vec<f64>);

impl Acc {
    fn row(&self, label: &str) {
        let mean = self.0.iter().sum::<f64>() / self.0.len().max(1) as f64;
        let min = self.0.iter().copied().fold(1.0, f64::min);
        let full = self.0.iter().filter(|&&r| r >= 1.0).count();
        println!(
            "| {label} | {} | {mean:.4} | {min:.2} | {full} |",
            self.0.len()
        );
    }
}

fn overlap(found: &[u64], truth: &[u64]) -> f64 {
    let t: HashSet<_> = truth.iter().collect();
    found.iter().filter(|f| t.contains(f)).count() as f64 / truth.len().max(1) as f64
}

fn open(p: &Paths, key: &mam_vector::ModelKey) -> FlatIndex {
    let store = EmbedStore::new(p.vectors()).unwrap();
    Generations::new(&store, key)
        .unwrap()
        .open_active(key, OpenMode::View)
        .unwrap()
        .expect("no active generation")
}

fn top_assets(hits: &[Hit], asset_of: &dyn Fn(u64) -> Option<u64>) -> Vec<u64> {
    group_by_asset(hits, asset_of)
        .iter()
        .take(10)
        .map(|h| h.asset_id)
        .collect()
}

pub fn run(p: &Paths, sc: Scale, queries: usize, similar: usize, threads: usize) {
    println!(
        "\n### Recall@10 (정답 = f32 전수 비교 `exact_topk_f32`, 대상 = 제품 f16 flat 세대 view)"
    );
    println!(
        "| 질의 종류 | 질의 수 | 평균 Recall@10 | 최소 | 만점 질의 수 |\n|---|---:|---:|---:|---:|"
    );
    let vis = open(p, &visual_key());
    vis.set_threads(threads);
    let mix = Mixture::visual();
    let rows = fill_visual(&mix, &sc, 1, sc.assets() + 1, threads);
    assert_eq!(rows.len() as u64, sc.visual() * VIS_DIM as u64);
    let ids: Vec<u64> = (1..=sc.visual()).collect();
    let qs = item_queries(&mix, queries);
    let of = |e: u64| Some(sc.visual_asset(e));
    let plan = plan_candidates(50);
    let params = SearchParams::default();

    // 임베딩 단위 (무필터)
    let mut emb = Acc(Vec::new());
    let mut asset_acc = Acc(Vec::new());
    for q in &qs {
        let truth = exact_topk_f32(&ids, &rows, VIS_DIM, q, 100, threads);
        let t10: Vec<u64> = truth.iter().take(10).map(|h| h.id).collect();
        let f: Vec<u64> = vis
            .search(q, 10, &params, None)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        emb.0.push(overlap(&f, &t10));
        let cands = vector_candidates(&vis, q, &plan, None, vis.len(), &of).unwrap();
        let fa: Vec<u64> = cands.iter().take(10).map(|h| h.asset_id).collect();
        asset_acc.0.push(overlap(&fa, &top_assets(&truth, &of)));
    }
    emb.row("시각 무필터, 임베딩(프레임) 단위");
    asset_acc.row("시각 무필터, 자산 단위 (vector_candidates)");

    // 필터 선적용 (자산 단위)
    for pct in [1u64, 10, 33] {
        let allowed = allowed_eids(&sc, pct);
        let (mut sub_rows, sub_ids) =
            (Vec::with_capacity(allowed.len() * VIS_DIM), allowed.clone());
        for &e in &allowed {
            let r = (e as usize - 1) * VIS_DIM;
            sub_rows.extend_from_slice(&rows[r..r + VIS_DIM]);
        }
        let mut acc = Acc(Vec::new());
        for q in &qs {
            let truth = exact_topk_f32(&sub_ids, &sub_rows, VIS_DIM, q, 100, threads);
            let c = vector_candidates(&vis, q, &plan, Some(&allowed), vis.len(), &of).unwrap();
            let fa: Vec<u64> = c.iter().take(10).map(|h| h.asset_id).collect();
            acc.0.push(overlap(&fa, &top_assets(&truth, &of)));
        }
        acc.row(&format!(
            "시각 허용 {pct}% ({} 벡터), 자산 단위",
            allowed.len()
        ));
    }

    // 유사 미디어: 질의 영상의 프레임 5개, 자기 자신 제외
    let mut sim = Acc(Vec::new());
    for (a, frames) in video_queries(&mix, &sc, similar) {
        let mut best: std::collections::HashMap<u64, f32> = std::collections::HashMap::new();
        for f in &frames {
            for h in exact_topk_f32(&ids, &rows, VIS_DIM, f, 100, threads) {
                let asset = sc.visual_asset(h.id);
                if asset != a {
                    let b = best.entry(asset).or_insert(f32::MIN);
                    *b = b.max(h.score);
                }
            }
        }
        let mut v: Vec<(u64, f32)> = best.into_iter().collect();
        v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
        let truth: Vec<u64> = v.iter().take(10).map(|x| x.0).collect();
        let q = SimilarQuery {
            frames: &frames,
            exclude: &[a],
            limit: 10,
            cursor: None,
            min_score: 0.0,
        };
        let page = find_similar(&vis, &q, &of).unwrap();
        let found: Vec<u64> = page.items.iter().map(|x| x.0).collect();
        sim.0.push(overlap(&found, &truth));
    }
    sim.row("유사 미디어 (프레임 5개, 자산 Top10)");
    drop((rows, ids));

    // 의미 384-d
    let sem = open(p, &semantic_key());
    sem.set_threads(threads);
    let smix = Mixture::semantic();
    let srows = fill_semantic(&smix, 1, sc.assets() + 1, threads);
    assert_eq!(srows.len() as u64, sc.assets() * SEM_DIM as u64);
    let sids: Vec<u64> = (1..=sc.assets()).map(|a| sc.semantic_eid(a)).collect();
    let mut sacc = Acc(Vec::new());
    for q in &item_queries(&smix, queries) {
        let truth = exact_topk_f32(&sids, &srows, SEM_DIM, q, 10, threads);
        let t: Vec<u64> = truth.iter().map(|h| h.id).collect();
        let f: Vec<u64> = sem
            .search(q, 10, &params, None)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        sacc.0.push(overlap(&f, &t));
    }
    sacc.row("의미 384-d 무필터, 임베딩=자산 단위");
}
