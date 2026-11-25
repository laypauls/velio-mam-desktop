//! T-vec-01~06: 모든 엔진 × 정밀도에 같은 계약을 적용한다.
//! 분류: Happy / Boundary / Validation / Concurrency(멱등) / Business rule.

mod common;

use common::*;
use mam_vector::{AnnIndex, Engine, FlatIndex, Quantization, VectorError, create_index};

fn data() -> Vec<Vec<f32>> {
    dataset(300, DIM, 7)
}

// ---------------------------------------------------------------- T-vec-01 (Happy)
#[test]
fn t_vec_01_add_then_search_returns_self_first() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        // Arrange
        let idx = build(&s, &d);
        // Act + Assert: 색인한 벡터로 질의하면 자기 id 가 1위
        for probe in [0usize, 57, 299] {
            let hits = idx.search(&d[probe], 5, &params(), None).unwrap();
            assert_eq!(hits[0].id, probe as u64 + 1, "{}", label(&s));
            assert!(
                hits[0].score > 0.97,
                "{} score {}",
                label(&s),
                hits[0].score
            );
        }
    }
}

// ---------------------------------------------------------------- T-vec-02 (Boundary)
#[test]
fn t_vec_02_boundaries() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        let l = label(&s);
        let empty = create_index(&s, 0).unwrap();
        // 빈 인덱스 -> 빈 결과
        assert!(
            empty.search(&d[0], 10, &params(), None).unwrap().is_empty(),
            "{l}"
        );
        assert!(empty.is_empty());

        let idx = build(&s, &d[..20]);
        // k=0, 상한 초과 -> InvalidInput
        for k in [0usize, mam_vector::MAX_K + 1] {
            assert!(
                matches!(
                    idx.search(&d[0], k, &params(), None),
                    Err(VectorError::InvalidInput(_))
                ),
                "{l} k={k}"
            );
        }
        // k > 건수 -> 건수만큼
        let hits = idx.search(&d[0], 100, &params(), None).unwrap();
        assert_eq!(hits.len(), 20, "{l}");
        // 차원 불일치 (질의, 추가)
        assert!(
            matches!(
                idx.search(&d[0][..DIM - 1], 3, &params(), None),
                Err(VectorError::DimensionMismatch { expected: DIM, found }) if found == DIM - 1
            ),
            "{l}"
        );
        assert!(matches!(
            idx.add(99, &d[0][..DIM - 1]),
            Err(VectorError::DimensionMismatch { .. })
        ));
        // 점수는 내림차순
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score), "{l}");
    }
}

// ---------------------------------------------------------------- T-vec-03 (Validation)
#[test]
fn t_vec_03_rejects_nan_inf_and_zero_vector() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        let idx = build(&s, &d[..10]);
        let mut nan = d[0].clone();
        nan[3] = f32::NAN;
        let mut inf = d[0].clone();
        inf[0] = f32::INFINITY;
        let zero = vec![0.0; DIM];
        for (name, v) in [("nan", &nan), ("inf", &inf), ("zero", &zero)] {
            assert!(
                matches!(idx.add(500, v), Err(VectorError::InvalidInput(_))),
                "{} add {name}",
                label(&s)
            );
            assert!(
                matches!(
                    idx.search(v, 3, &params(), None),
                    Err(VectorError::InvalidInput(_))
                ),
                "{} search {name}",
                label(&s)
            );
        }
        // 거부된 벡터는 인덱스에 남지 않는다
        assert_eq!(idx.len(), 10, "{}", label(&s));
    }
}

// ---------------------------------------------------------------- T-vec-04 (Concurrency: 멱등)
#[test]
fn t_vec_04_add_same_id_replaces_and_remove_missing_is_ok() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        let l = label(&s);
        let idx = build(&s, &d[..50]);
        // 같은 id 에 다른 벡터를 두 번 add -> 1개 (교체)
        idx.add(10, &d[200]).unwrap();
        idx.add(10, &d[200]).unwrap();
        assert_eq!(idx.len(), 50, "{l}");
        let hits = idx.search(&d[200], 1, &params(), None).unwrap();
        assert_eq!(hits[0].id, 10, "{l}: 교체된 벡터로 찾아져야 한다");
        // 원래 벡터(d[9], id 10)로는 더 이상 id 10 이 1위가 아니다
        let old = idx.search(&d[9], 1, &params(), None).unwrap();
        assert_ne!(old[0].id, 10, "{l}: 이전 벡터가 남아 있다");
        // 없는 id remove -> 오류 없음 / false
        assert!(!idx.remove(9_999).unwrap(), "{l}");
        // 두 번 remove
        assert!(idx.remove(10).unwrap(), "{l}");
        assert!(!idx.remove(10).unwrap(), "{l}");
        assert_eq!(idx.len(), 49, "{l}");
    }
}

// ---------------------------------------------------------------- T-vec-05 (Business rule)
#[test]
fn t_vec_05_removed_vectors_never_appear() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        let l = label(&s);
        let idx = build(&s, &d);
        let removed: Vec<u64> = (1..=300).filter(|i| i % 3 == 0).collect();
        for id in &removed {
            assert!(idx.remove(*id).unwrap(), "{l}");
        }
        assert_eq!(idx.len(), 200, "{l}");
        for probe in [2usize, 5, 100, 250] {
            let hits = idx.search(&d[probe], 50, &params(), None).unwrap();
            assert!(hits.iter().all(|h| !removed.contains(&h.id)), "{l}");
        }
        // 삭제한 벡터 자신으로 질의해도 자기 id 는 없다
        let hits = idx.search(&d[2], 10, &params(), None).unwrap(); // id 3 삭제됨
        assert!(hits.iter().all(|h| h.id != 3), "{l}");
        // 삭제 후 추가 삽입도 정상
        idx.add(3, &d[2]).unwrap();
        assert_eq!(
            idx.search(&d[2], 1, &params(), None).unwrap()[0].id,
            3,
            "{l}"
        );
    }
}

// ---------------------------------------------------------------- T-vec-06 (Business rule: 필터)
/// 허용 판정: id 를 4개 "볼륨"으로, 2개 "유형"으로 나눈다. 볼륨 1·2 의 이미지(짝수 id)만 허용.
fn allowed(id: u64) -> bool {
    let volume = id % 4;
    let is_image = id.is_multiple_of(2);
    (volume == 0 || volume == 2) && is_image
}

#[test]
fn t_vec_06_filter_contract_f1_f2_f3() {
    common::affinity::exclude_inconsistent_cores();
    let d = data();
    for s in matrix() {
        let l = label(&s);
        let idx = build(&s, &d);
        let q = &d[17];
        let allow_ids: Vec<u64> = (1..=300).filter(|&i| allowed(i)).collect();

        // F1: 검색 중 필터
        let f1 = idx.search(q, 20, &params(), Some(&allowed)).unwrap();
        assert!(!f1.is_empty(), "{l}");
        assert!(f1.iter().all(|h| allowed(h.id)), "{l} F1 leaked");
        // F2: 후필터 + 후보 확대
        let f2 = mam_vector::search_post_filter(&*idx, q, 20, &params(), &allowed, 4).unwrap();
        assert!(f2.hits.iter().all(|h| allowed(h.id)), "{l} F2 leaked");
        assert_eq!(f2.hits.len(), 20, "{l}: 확대로 k 를 채워야 한다");
        // 허용 집합이 비면 빈 결과 (F1, F2)
        let none = |_: u64| false;
        assert!(
            idx.search(q, 10, &params(), Some(&none))
                .unwrap()
                .is_empty(),
            "{l}"
        );
        let f2e = mam_vector::search_post_filter(&*idx, q, 10, &params(), &none, 4).unwrap();
        assert!(f2e.hits.is_empty(), "{l}");

        // 정확도: 전수 비교 정답 중 허용된 것과의 Recall
        let truth: Vec<u64> = naive_topk(&d, q, 300)
            .into_iter()
            .filter(|&i| allowed(i))
            .take(20)
            .collect();
        let ids1: Vec<u64> = f1.iter().map(|h| h.id).collect();
        let tol = if s.quantization == Quantization::I8 {
            0.8
        } else {
            0.9
        };
        assert!(
            recall(&ids1, &truth) >= tol,
            "{l} F1 recall {}",
            recall(&ids1, &truth)
        );

        // F3: 전수 비교 엔진에 허용 집합만 (flat 만)
        if s.engine == Engine::Flat {
            let flat = FlatIndex::new(DIM, s.quantization);
            for (i, v) in d.iter().enumerate() {
                flat.add(i as u64 + 1, v).unwrap();
            }
            let f3 = flat.search_subset(q, 20, &allow_ids).unwrap();
            assert!(f3.iter().all(|h| allowed(h.id)), "{l} F3 leaked");
            assert_eq!(f3.len(), 20);
            assert!(flat.search_subset(q, 20, &[]).unwrap().is_empty());
            // 인덱스에 없는 id 는 무시
            assert!(flat.search_subset(q, 5, &[999_999]).unwrap().is_empty());
        }
    }
}
