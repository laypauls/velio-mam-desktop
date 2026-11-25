//! T-det-*: 같은 호출은 점수 비트와 순서가 항상 같아야 한다 — 스레드 수·분할·정렬·적재 방식과 무관.
//! 분류: Happy / Boundary(정렬 오프셋, 동점) / Concurrency(스레드 수) / Business rule(동점은 id 오름차순).

mod common;

use common::Rng;
use mam_vector::{
    AnnIndex, FlatIndex, Hit, OpenMode, Quantization, SearchParams,
    kernel::{dot_f16, dot_f32, dot_i8},
    normalized,
};

const DIM: usize = 512;
const N: usize = 50_000;
const K: usize = 300;
const RUNS: usize = 20;

fn build(quant: Quantization, n: usize, dim: usize, seed: u64) -> FlatIndex {
    let mut rng = Rng(seed);
    let idx = FlatIndex::with_capacity(dim, quant, n);
    for i in 0..n {
        let v: Vec<f32> = (0..dim).map(|_| rng.signed()).collect();
        idx.add(i as u64 * 3 + 1, &normalized(&v)).unwrap();
    }
    idx
}

fn query(dim: usize, seed: u64) -> Vec<f32> {
    let mut rng = Rng(seed);
    (0..dim).map(|_| rng.signed()).collect()
}

fn bits(h: &[Hit]) -> Vec<(u64, u32)> {
    h.iter().map(|x| (x.id, x.score.to_bits())).collect()
}

fn thread_counts() -> Vec<usize> {
    let all = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut t = vec![1, 2, 3, 12, all];
    t.sort_unstable();
    t.dedup();
    t
}

const QUANTS: [Quantization; 3] = [Quantization::F16, Quantization::F32, Quantization::I8];

// ---------------------------------------------------------------- T-det-01 (Concurrency)
#[test]
fn t_det_01_search_is_bit_identical_across_runs_and_thread_counts() {
    common::affinity::exclude_inconsistent_cores();
    for quant in QUANTS {
        // Arrange
        let idx = build(quant, N, DIM, 11);
        let q = query(DIM, 12);
        let p = SearchParams::default();
        let mut reference: Option<Vec<(u64, u32)>> = None;
        for t in thread_counts() {
            idx.set_threads(t);
            for run in 0..RUNS {
                // Act
                let got = bits(&idx.search(&q, K, &p, None).unwrap());
                // Assert
                let r = reference.get_or_insert_with(|| got.clone());
                assert_eq!(&got, r, "{quant:?} threads={t} run={run}");
            }
        }
    }
}

// ---------------------------------------------------------------- T-det-02 (Concurrency)
#[test]
fn t_det_02_subset_and_filtered_search_are_bit_identical() {
    common::affinity::exclude_inconsistent_cores();
    for quant in QUANTS {
        let idx = build(quant, N, DIM, 21);
        let q = query(DIM, 22);
        let p = SearchParams::default();
        // 일부 id 는 인덱스에 없다 (건너뛴다), 순서는 섞여 있다
        let allowed: Vec<u64> = (0..N as u64 * 2)
            .rev()
            .map(|i| i * 3 + 1)
            .step_by(2)
            .collect();
        let pred = |id: u64| !id.is_multiple_of(5);
        let (mut ref_sub, mut ref_flt): (Option<Vec<_>>, Option<Vec<_>>) = (None, None);
        for t in thread_counts() {
            idx.set_threads(t);
            for run in 0..RUNS {
                let sub = bits(&idx.search_subset(&q, K, &allowed).unwrap());
                let flt = bits(&idx.search(&q, K, &p, Some(&pred)).unwrap());
                assert_eq!(
                    &sub,
                    ref_sub.get_or_insert_with(|| sub.clone()),
                    "{quant:?} subset t={t} r={run}"
                );
                assert_eq!(
                    &flt,
                    ref_flt.get_or_insert_with(|| flt.clone()),
                    "{quant:?} filter t={t} r={run}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- T-det-03 (Concurrency)
#[test]
fn t_det_03_search_multi_is_bit_identical() {
    common::affinity::exclude_inconsistent_cores();
    for quant in QUANTS {
        let idx = build(quant, N, DIM, 31);
        let qs: Vec<Vec<f32>> = (0..4).map(|i| query(DIM, 40 + i)).collect();
        let mut reference: Option<Vec<(u64, u32)>> = None;
        for t in thread_counts() {
            idx.set_threads(t);
            for run in 0..RUNS {
                let got = bits(&idx.search_multi(&qs, K, None).unwrap());
                assert_eq!(
                    &got,
                    reference.get_or_insert_with(|| got.clone()),
                    "{quant:?} t={t} r={run}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- T-det-04 (Business rule)
#[test]
fn t_det_04_equal_scores_break_ties_by_ascending_id_everywhere() {
    common::affinity::exclude_inconsistent_cores();
    for quant in QUANTS {
        // Arrange: 같은 벡터를 id 역순으로 넣는다 -> 점수가 전부 같다
        let dim = 32;
        let n = 40_000; // 병렬 임계 이상
        let v = normalized(&query(dim, 5));
        let idx = FlatIndex::new(dim, quant);
        for i in (0..n as u64).rev() {
            idx.add(i + 1, &v).unwrap();
        }
        let allowed: Vec<u64> = (1..=n as u64).rev().collect();
        for t in [1, 2, 7] {
            idx.set_threads(t);
            // Act
            let a = idx.search(&v, 50, &SearchParams::default(), None).unwrap();
            let b = idx.search_subset(&v, 50, &allowed).unwrap();
            let c = idx.search_multi(&[v.clone(), v.clone()], 50, None).unwrap();
            // Assert
            for (name, h) in [("search", a), ("subset", b), ("multi", c)] {
                let ids: Vec<u64> = h.iter().map(|x| x.id).collect();
                assert_eq!(
                    ids,
                    (1..=50).collect::<Vec<u64>>(),
                    "{quant:?} {name} t={t}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- T-det-05 (Boundary: 정렬/적재 방식)
#[test]
fn t_det_05_view_and_load_modes_give_identical_bits() {
    common::affinity::exclude_inconsistent_cores();
    for quant in QUANTS {
        let dim = 48; // 16 의 배수가 아닌 꼬리
        let idx = build(quant, 5_000, dim, 51);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flat.bin");
        idx.save_file(&path).unwrap();
        let load = FlatIndex::open(&path, dim, quant, OpenMode::Load).unwrap();
        let view = FlatIndex::open(&path, dim, quant, OpenMode::View).unwrap();
        let q = query(dim, 52);
        let p = SearchParams::default();
        let want = bits(&idx.search(&q, 500, &p, None).unwrap());
        for t in [1, 4] {
            for i in [&idx, &load, &view] {
                i.set_threads(t);
                assert_eq!(
                    bits(&i.search(&q, 500, &p, None).unwrap()),
                    want,
                    "{quant:?} t={t}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- T-det-06 (Boundary: 슬라이스 시작 오프셋)
#[test]
fn t_det_06_kernels_ignore_slice_alignment_and_length_tails() {
    common::affinity::exclude_inconsistent_cores();
    let mut rng = Rng(61);
    let pool_f: Vec<f32> = (0..2_000).map(|_| rng.signed()).collect();
    let pool_h: Vec<u16> = pool_f
        .iter()
        .map(|&x| half::f16::from_f32(x).to_bits())
        .collect();
    let pool_i: Vec<i8> = pool_f.iter().map(|&x| (x * 127.0) as i8).collect();
    for len in [1usize, 7, 15, 16, 17, 31, 33, 48, 100, 512, 513] {
        let qf = &pool_f[..len];
        let (r32, r16, r8) = (
            dot_f32(qf, &pool_f[1000..1000 + len]).to_bits(),
            dot_f16(qf, &pool_h[1000..1000 + len]).to_bits(),
            dot_i8(&pool_i[..len], &pool_i[1000..1000 + len]),
        );
        // 같은 내용을 여러 시작 오프셋(주소 정렬)으로 복사해 계산한다
        for off in 0..40 {
            let mut f = vec![0f32; off + len];
            f[off..].copy_from_slice(&pool_f[1000..1000 + len]);
            let mut h = vec![0u16; off + len];
            h[off..].copy_from_slice(&pool_h[1000..1000 + len]);
            let mut b = vec![0i8; off + len];
            b[off..].copy_from_slice(&pool_i[1000..1000 + len]);
            assert_eq!(
                dot_f32(qf, &f[off..]).to_bits(),
                r32,
                "f32 len={len} off={off}"
            );
            assert_eq!(
                dot_f16(qf, &h[off..]).to_bits(),
                r16,
                "f16 len={len} off={off}"
            );
            assert_eq!(
                dot_i8(&pool_i[..len], &b[off..]),
                r8,
                "i8 len={len} off={off}"
            );
        }
    }
}

// ---------------------------------------------------------------- T-det-07 (Business rule: 고정 누적 순서)
/// 커널 문서의 순서를 그대로 옮긴 기준값 — 16 레인 누적, 레인 순 합산, 꼬리 순차 가산.
fn reference_f32(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 16];
    let (ca, cb) = (a.chunks_exact(16), b.chunks_exact(16));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..16 {
            acc[i] += std::hint::black_box(x[i]) * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[test]
fn t_det_07_kernel_matches_fixed_order_reference_bit_for_bit() {
    common::affinity::exclude_inconsistent_cores();
    let mut rng = Rng(71);
    for len in [1usize, 15, 16, 17, 48, 100, 512, 513] {
        for _ in 0..50 {
            let a: Vec<f32> = (0..len).map(|_| rng.signed()).collect();
            let b: Vec<f32> = (0..len).map(|_| rng.signed()).collect();
            assert_eq!(
                dot_f32(&a, &b).to_bits(),
                reference_f32(&a, &b).to_bits(),
                "len={len}"
            );
        }
    }
}

// ---------------------------------------------------------------- T-det-08 (Business rule: 자산 묶기 동점)
#[test]
fn t_det_08_group_by_asset_tie_picks_smallest_embedding_id_regardless_of_input_order() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 자산 7 의 프레임 3 개가 같은 점수 — 입력 순서를 바꿔도 같은 결과여야 한다
    let hits = [
        Hit { id: 30, score: 0.5 },
        Hit { id: 10, score: 0.5 },
        Hit { id: 20, score: 0.5 },
        Hit { id: 99, score: 0.4 },
    ];
    let asset_of = |e: u64| Some(if e == 99 { 8 } else { 7 });
    for perm in [[0, 1, 2, 3], [2, 1, 0, 3], [1, 0, 3, 2], [3, 2, 1, 0]] {
        let input: Vec<Hit> = perm.iter().map(|&i| hits[i]).collect();
        // Act
        let got = mam_vector::group_by_asset(&input, &asset_of);
        // Assert
        let summary: Vec<(u64, u64)> = got.iter().map(|a| (a.asset_id, a.embedding_id)).collect();
        assert_eq!(summary, vec![(7, 10), (8, 99)], "perm {perm:?}");
    }
}
