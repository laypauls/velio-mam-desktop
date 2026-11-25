//! 저장 비용 측정 (수동): `MAM_PERSIST_N=200000 cargo test -p mam-vector --release --test persist_cost -- --ignored --nocapture`.
//! 임시 디렉터리는 `MAM_PERSIST_DIR` (기본: 시스템 임시) 아래에 만들고 끝나면 지운다.
//! 이전(제자리 `save_index`)과 이후(`save_index_version`)의 저장 시간, 새 버전을 View 로 여는 시간을 낸다.

mod common;

use std::time::Instant;

use common::{DIM, Rng, spec};
use mam_vector::{
    AnnIndex, Engine, FlatIndex, OpenMode, Quantization, data_file_versions, open_index,
    save_index, save_index_version,
};

const RUNS: usize = 5;

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

#[test]
#[ignore = "manual measurement"]
fn persist_cost() {
    let n: usize = std::env::var("MAM_PERSIST_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(200_000);
    let base = std::env::var("MAM_PERSIST_DIR")
        .unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    std::fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::TempDir::new_in(&base).unwrap();
    let s = spec(Engine::Flat, Quantization::F16);
    let idx = FlatIndex::with_capacity(512, Quantization::F16, n);
    let mut rng = Rng(1);
    for id in 1..=n as u64 {
        let v: Vec<f32> = (0..512).map(|_| rng.signed()).collect();
        idx.add(id, &mv(&v)).unwrap();
    }
    let mut s512 = s.clone();
    s512.dimension = 512;
    let _ = DIM;

    let (mut before, mut after, mut open) = (vec![], vec![], vec![]);
    for _ in 0..RUNS {
        let t = Instant::now();
        save_index(tmp.path(), &idx, &s512).unwrap();
        before.push(t.elapsed().as_secs_f64());
    }
    for _ in 0..RUNS {
        let t = Instant::now();
        save_index_version(tmp.path(), &idx, &s512).unwrap();
        after.push(t.elapsed().as_secs_f64());
        let t = Instant::now();
        let v = open_index(tmp.path(), &s512, OpenMode::View).unwrap();
        open.push(t.elapsed().as_secs_f64());
        assert_eq!(v.len(), n);
    }
    let mb = std::fs::metadata(tmp.path().join("index.flat"))
        .map(|m| m.len())
        .unwrap_or(0) as f64
        / 1e6;
    println!(
        "n={n} dim=512 f16 file={mb:.0}MB versions={:?}\n  in-place save_index        p50={:.3}s max={:.3}s\n  save_index_version         p50={:.3}s max={:.3}s\n  open View (new version)    p50={:.4}s max={:.4}s",
        data_file_versions(tmp.path()),
        pct(&mut before.clone(), 0.5),
        pct(&mut before, 1.0),
        pct(&mut after.clone(), 0.5),
        pct(&mut after, 1.0),
        pct(&mut open.clone(), 0.5),
        pct(&mut open, 1.0),
    );
}

/// 정규화 없이 f32 그대로 (측정용 — 내용은 무관).
fn mv(v: &[f32]) -> Vec<f32> {
    mam_vector::normalized(v)
}
