//! T-vec-10·11·12·13: 다중 프레임 묶기, 모델 교체, 동시성, 전수 비교 정답 검증.
//! 분류: Business rule / Concurrency / Boundary / External failure.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use mam_vector::kernel::{dot_f16, dot_f32, dot_i8, f16_bits_to_f32, quantize_i8};
use mam_vector::{
    AnnIndex, AssetHit, Engine, FlatIndex, Hit, IndexRoot, OpenMode, Quantization, VectorError,
    exact_topk_f32, group_by_asset, open_index, save_index, search_assets,
};

// ---------------------------------------------------------------- T-vec-10 (Business rule)
#[test]
fn t_vec_10_group_by_asset_dedups_and_uses_max_score() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 자산 7 의 프레임 3개, 자산 9 의 프레임 1개, 자산 없는(고아) 벡터 1개
    let hits = [
        Hit { id: 1, score: 0.90 },
        Hit { id: 2, score: 0.95 },
        Hit { id: 3, score: 0.80 },
        Hit { id: 4, score: 0.92 },
        Hit {
            id: 99,
            score: 0.99,
        }, // 고아
        Hit { id: 5, score: 0.70 },
    ];
    let asset_of = |id: u64| match id {
        1..=3 => Some(7),
        4 => Some(9),
        5 => Some(11),
        _ => None,
    };
    // Act
    let g = group_by_asset(&hits, &asset_of);
    // Assert: 중복 없음, 자산 점수 = 프레임 최대, 점수 내림차순, 고아 제외
    assert_eq!(
        g,
        vec![
            AssetHit {
                asset_id: 7,
                score: 0.95,
                embedding_id: 2
            },
            AssetHit {
                asset_id: 9,
                score: 0.92,
                embedding_id: 4
            },
            AssetHit {
                asset_id: 11,
                score: 0.70,
                embedding_id: 5
            },
        ]
    );
    assert!(group_by_asset(&[], &asset_of).is_empty());
}

#[test]
fn t_vec_10b_search_assets_grows_k_until_enough_assets() {
    common::affinity::exclude_inconsistent_cores();
    // 자산마다 프레임 5개(서로 아주 가까움) -> 상위 k 를 한 자산이 독차지한다
    let assets = 40usize;
    let base = dataset(assets, DIM, 21);
    let mut rng = Rng(5);
    let mut all: Vec<Vec<f32>> = Vec::new();
    for a in &base {
        for _ in 0..5 {
            let v: Vec<f32> = a.iter().map(|x| x + 0.01 * rng.signed()).collect();
            all.push(v);
        }
    }
    let asset_of = |id: u64| Some((id - 1) / 5 + 1);
    for s in matrix() {
        let l = label(&s);
        let idx = build(&s, &all);
        let r = search_assets(&*idx, &base[3], 10, 10, &params(), None, &asset_of).unwrap();
        assert_eq!(r.hits.len(), 10, "{l}");
        let mut ids: Vec<u64> = r.hits.iter().map(|h| h.asset_id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 10, "{l}: asset_id 중복");
        assert_eq!(r.hits[0].asset_id, 4, "{l}"); // base[3] 의 자산 = 4
        assert!(r.k_used >= 10 && r.rounds >= 1, "{l}");
        assert!(r.hits.windows(2).all(|w| w[0].score >= w[1].score), "{l}");
        // 자산 수보다 많이 요청 -> 있는 만큼만 (무한 루프 없음)
        let r = search_assets(&*idx, &base[3], 100, 50, &params(), None, &asset_of).unwrap();
        assert!(r.hits.len() <= assets, "{l}");
    }
}

// ---------------------------------------------------------------- T-vec-11 (Business rule: 모델 교체)
fn model_spec(version: &str) -> mam_vector::IndexSpec {
    let mut s = spec(Engine::Flat, Quantization::F16);
    s.model_version = version.into();
    s
}

#[test]
fn t_vec_11_model_swap_builds_new_activates_then_retires_old() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(100, DIM, 31);
    let root_dir = tempfile::tempdir().unwrap();
    let root = IndexRoot::new(root_dir.path().join("vectors")).unwrap();
    let (old_spec, new_spec) = (model_spec("1"), model_spec("2"));

    // 첫 세대
    root.build_generation("v1", |dir| {
        save_index(dir, &*build(&old_spec, &d), &old_spec)
    })
    .unwrap();
    assert_eq!(root.activate("v1", &old_spec).unwrap(), None);
    let old_view = open_index(&root.dir("v1"), &old_spec, OpenMode::View).unwrap();

    // 새 모델 세대를 새 디렉터리에 구축 -> 활성 교체
    root.build_generation("v2", |dir| {
        save_index(dir, &*build(&new_spec, &d), &new_spec)
    })
    .unwrap();
    assert_eq!(
        root.activate("v2", &new_spec).unwrap().as_deref(),
        Some("v1")
    );
    assert_eq!(root.active().unwrap().as_deref(), Some("v2"));

    // 활성 세대는 지울 수 없다
    assert!(matches!(
        root.retire("v2"),
        Err(VectorError::InvalidInput(_))
    ));

    drop(old_view);
    root.retire("v1").unwrap();
    assert!(!root.dir("v1").exists());
    // 이미 없는 세대 retire 는 멱등
    root.retire("v1").unwrap();
    // 새 세대는 정상적으로 열린다
    assert_eq!(
        open_index(&root.dir("v2"), &new_spec, OpenMode::View)
            .unwrap()
            .len(),
        100
    );
}

#[test]
fn t_vec_11b_failed_build_keeps_old_index_active() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(50, DIM, 32);
    let root_dir = tempfile::tempdir().unwrap();
    let root = IndexRoot::new(root_dir.path()).unwrap();
    let old_spec = model_spec("1");
    root.build_generation("v1", |dir| {
        save_index(dir, &*build(&old_spec, &d), &old_spec)
    })
    .unwrap();
    root.activate("v1", &old_spec).unwrap();

    // 구축 중 실패 -> 새 디렉터리 정리, 포인터 유지
    let r = root.build_generation("v2", |dir| {
        std::fs::write(dir.join("partial"), b"x")?;
        Err(VectorError::Engine("simulated build failure".into()))
    });
    assert!(r.is_err());
    assert!(!root.dir("v2").exists());
    assert_eq!(root.active().unwrap().as_deref(), Some("v1"));
    assert_eq!(
        open_index(&root.dir("v1"), &old_spec, OpenMode::Load)
            .unwrap()
            .len(),
        50
    );

    // 메타가 없는(미완성) 세대나 다른 설정의 세대는 활성화할 수 없다
    std::fs::create_dir_all(root.dir("v3")).unwrap();
    assert!(matches!(
        root.activate("v3", &model_spec("3")),
        Err(VectorError::IndexMismatch(_))
    ));
    assert!(matches!(
        root.activate("v1", &model_spec("9")),
        Err(VectorError::IndexMismatch(_))
    ));
    assert_eq!(root.active().unwrap().as_deref(), Some("v1"));
    // 이미 있는 세대 이름으로는 만들 수 없다
    assert!(root.build_generation("v1", |_| Ok(())).is_err());
}

// ---------------------------------------------------------------- T-vec-12 (Concurrency)
#[test]
fn t_vec_12_search_while_adding_is_safe() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(600, DIM, 41);
    for s in matrix() {
        let l = label(&s);
        let idx = build(&s, &d[..300]);
        let stop = AtomicBool::new(false);
        let (mut total, mut misses) = (0usize, 0usize);
        std::thread::scope(|sc| {
            // 검색 스레드 2개: 처음 300개 중 하나로 질의. 결과는 항상 유효해야 하고(범위 내 id, 중복 없음, 정렬),
            // 전수 비교는 자기 자신이 항상 1위, HNSW 는 삽입 중 그래프가 갱신되는 동안 가끔 놓칠 수 있다(근사).
            let searchers: Vec<_> = (0..2)
                .map(|t| {
                    let (idx, d, stop) = (&idx, &d, &stop);
                    sc.spawn(move || {
                        let (mut n, mut miss) = (0usize, 0usize);
                        while !stop.load(Ordering::Relaxed) || n < 20 {
                            let probe = (n * 7 + t * 13) % 300;
                            let hits = idx.search(&d[probe], 5, &params(), None).unwrap();
                            assert!(!hits.is_empty());
                            assert!(hits.iter().all(|h| (1..=600).contains(&h.id)));
                            let mut ids: Vec<u64> = hits.iter().map(|h| h.id).collect();
                            ids.sort();
                            ids.dedup();
                            assert_eq!(ids.len(), hits.len(), "중복 id");
                            assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));
                            if hits[0].id != probe as u64 + 1 {
                                miss += 1;
                            }
                            n += 1;
                        }
                        (n, miss)
                    })
                })
                .collect();
            // 쓰기 스레드: 나머지 300개 추가
            for (i, v) in d.iter().enumerate().skip(300) {
                idx.add(i as u64 + 1, v).unwrap();
            }
            stop.store(true, Ordering::Relaxed);
            for h in searchers {
                let (n, m) = h.join().unwrap();
                total += n;
                misses += m;
            }
        });
        eprintln!("{l}: 동시 add 중 질의 {total}회, 자기 자신 1위 놓침 {misses}회");
        if s.engine == Engine::Flat {
            assert_eq!(misses, 0, "{l}: 전수 비교는 놓치면 안 된다");
        } else {
            assert!(misses * 10 <= total, "{l}: 놓침 {misses}/{total}");
        }
        // 동시 삽입이 끝난 뒤에는 정상 상태: 모든 건수, 새 벡터도 찾아진다
        assert_eq!(idx.len(), 600, "{l}");
        assert_eq!(
            idx.search(&d[599], 1, &params(), None).unwrap()[0].id,
            600,
            "{l}"
        );
    }
}

// ---------------------------------------------------------------- T-vec-13 (Boundary: 정답 검증)
#[test]
fn t_vec_13_flat_f32_matches_naive_loop() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(500, DIM, 51);
    let flat = build(&spec(Engine::Flat, Quantization::F32), &d);
    for probe in [0usize, 123, 499] {
        let q = &d[probe];
        let got: Vec<u64> = flat
            .search(q, 25, &params(), None)
            .unwrap()
            .iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(got, naive_topk(&d, q, 25), "probe {probe}");
    }
    // 점수 값도 단순 계산과 일치
    let h = flat.search(&d[10], 3, &params(), None).unwrap();
    let nq = mam_vector::normalized(&d[10]);
    let nv = mam_vector::normalized(&d[h[1].id as usize - 1]);
    let expect: f32 = nq.iter().zip(&nv).map(|(a, b)| a * b).sum();
    assert!((h[1].score - expect).abs() < 1e-5);
}

#[test]
fn t_vec_13b_exact_topk_f32_and_kernels_match_scalar() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(300, DIM, 52);
    let ids: Vec<u64> = (1..=300).map(|i| i * 10).collect(); // 연속이 아닌 id
    let rows: Vec<f32> = d.iter().flat_map(|v| mam_vector::normalized(v)).collect();
    for threads in [1, 4] {
        let got: Vec<u64> = exact_topk_f32(&ids, &rows, DIM, &d[5], 10, threads)
            .iter()
            .map(|h| h.id)
            .collect();
        let want: Vec<u64> = naive_topk(&d, &d[5], 10).iter().map(|i| i * 10).collect();
        assert_eq!(got, want, "threads={threads}");
    }
    // 커널: 길이가 16 의 배수가 아닌 경우(꼬리 처리)도 스칼라와 일치
    for len in [1usize, 15, 16, 17, 33, 100] {
        let a: Vec<f32> = (0..len).map(|i| (i as f32 * 0.37).sin()).collect();
        let b: Vec<f32> = (0..len).map(|i| (i as f32 * 0.91).cos()).collect();
        let want: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        assert!((dot_f32(&a, &b) - want).abs() < 1e-4, "f32 len {len}");
        let h: Vec<u16> = b
            .iter()
            .map(|&x| half::f16::from_f32(x).to_bits())
            .collect();
        let want16: f32 = a.iter().zip(&h).map(|(x, y)| x * f16_bits_to_f32(*y)).sum();
        assert!((dot_f16(&a, &h) - want16).abs() < 1e-4, "f16 len {len}");
        let (mut qa, mut qb) = (vec![0i8; len], vec![0i8; len]);
        quantize_i8(&a, &mut qa);
        quantize_i8(&b, &mut qb);
        let want8: i32 = qa.iter().zip(&qb).map(|(x, y)| *x as i32 * *y as i32).sum();
        assert_eq!(dot_i8(&qa, &qb), want8, "i8 len {len}");
    }
    // f16 비트 변환: 0, 음수, 비정규수
    for x in [0.0f32, -0.5, 1.0, 1e-6, -3.0e-5, 0.0123] {
        let bits = half::f16::from_f32(x).to_bits();
        assert_eq!(
            f16_bits_to_f32(bits),
            half::f16::from_bits(bits).to_f32(),
            "x={x}"
        );
    }
}

#[test]
fn t_vec_13c_flat_quantized_recall_and_swap_remove_consistency() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(500, DIM, 53);
    let exact = build(&spec(Engine::Flat, Quantization::F32), &d);
    for (q, min) in [(Quantization::F16, 0.99), (Quantization::I8, 0.9)] {
        let idx = build(&spec(Engine::Flat, q), &d);
        let mut total = 0.0;
        for probe_v in d.iter().take(50) {
            let t: Vec<u64> = exact
                .search(probe_v, 10, &params(), None)
                .unwrap()
                .iter()
                .map(|h| h.id)
                .collect();
            let g: Vec<u64> = idx
                .search(probe_v, 10, &params(), None)
                .unwrap()
                .iter()
                .map(|h| h.id)
                .collect();
            total += recall(&g, &t);
        }
        assert!(total / 50.0 >= min, "{q:?} recall {}", total / 50.0);
    }
    // swap-remove 후에도 남은 id 가 자기 벡터로 찾아진다 (id <-> 슬롯 표 일관성)
    let idx = FlatIndex::new(DIM, Quantization::F32);
    for (i, v) in d.iter().enumerate() {
        idx.add(i as u64 + 1, v).unwrap();
    }
    for id in [1u64, 250, 500, 7, 499] {
        idx.remove(id).unwrap();
    }
    for id in [2u64, 100, 498, 8] {
        let h = idx.search(&d[id as usize - 1], 1, &params(), None).unwrap();
        assert_eq!(h[0].id, id);
        assert!(idx.contains(id));
    }
    assert!(!idx.contains(250));
}

/// Windows 에서 메모리 매핑 중인 인덱스 파일의 삭제·이름 변경 동작을 엔진별로 관찰한다.
/// Rust `std::fs` 는 POSIX 삭제 의미론을 쓰므로 삭제가 "성공"할 수 있고, C++ 엔진(USearch)이 연 매핑은 다를 수 있다.
/// 단언하는 계약: view 핸들을 drop 한 뒤에는 삭제 + 같은 경로 재생성이 항상 된다.
#[test]
fn t_vec_11c_mapped_file_delete_behavior_per_engine() {
    common::affinity::exclude_inconsistent_cores();
    let d = dataset(100, DIM, 33);
    for s in matrix()
        .into_iter()
        .filter(|s| s.quantization == Quantization::F16)
    {
        let l = label(&s);
        let dir = tempfile::tempdir().unwrap();
        save_index(dir.path(), &*build(&s, &d), &s).unwrap();
        let data = s.engine.data_path(dir.path());
        let view = open_index(dir.path(), &s, OpenMode::View).unwrap();

        let moved = dir.path().join("moved");
        let renamed = std::fs::rename(&data, &moved);
        eprintln!(
            "{l}: rename while mapped -> {:?}",
            renamed.as_ref().map_err(|e| e.to_string())
        );
        if renamed.is_ok() {
            std::fs::rename(&moved, &data).unwrap();
        }
        let removed = std::fs::remove_file(&data);
        eprintln!(
            "{l}: remove_file while mapped -> {:?}",
            removed.as_ref().map_err(|e| e.to_string())
        );
        let recreated = std::fs::write(&data, b"x");
        eprintln!(
            "{l}: recreate same path while mapped -> {:?}",
            recreated.as_ref().map_err(|e| e.to_string())
        );

        drop(view);
        let _ = std::fs::remove_file(&data);
        std::fs::write(&data, b"x").unwrap_or_else(|e| panic!("{l}: 해제 후에도 재생성 불가: {e}"));
        std::fs::remove_file(&data).unwrap_or_else(|e| panic!("{l}: 해제 후에도 삭제 불가: {e}"));
    }
}
