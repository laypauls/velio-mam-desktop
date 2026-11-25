//! T-fus-01·02·03 (+ degraded): RRF 결합. 분류: Happy / Business rule / Boundary.

use mam_vector::{AssetHit, FusedHit, RrfParams, fuse_hybrid, fuse_rrf};

fn vh(asset_id: u64, score: f32) -> AssetHit {
    AssetHit {
        asset_id,
        score,
        embedding_id: asset_id * 10,
    }
}

fn ids(h: &[FusedHit]) -> Vec<u64> {
    h.iter().map(|x| x.asset_id).collect()
}

// ---------------------------------------------------------------- T-fus-01 (Happy)
#[test]
fn t_fus_01_rrf_matches_hand_calculation_and_keeps_single_source_hits() {
    // Arrange: 키워드 [10, 20, 30], 벡터 [20, 40, 10]
    let kw = [10, 20, 30];
    let vec = [vh(20, 0.9), vh(40, 0.8), vh(10, 0.7)];
    // Act
    let r = fuse_rrf(&kw, &vec, &RrfParams::default());
    // Assert: 20 = 1/62 + 1/61, 10 = 1/61 + 1/63, 40 = 1/62, 30 = 1/63
    let s = |a: f64, b: f64| 1.0 / (60.0 + a) + 1.0 / (60.0 + b);
    assert_eq!(ids(&r), vec![20, 10, 40, 30]);
    assert!((r[0].rrf - s(2.0, 1.0)).abs() < 1e-12);
    assert!((r[1].rrf - s(1.0, 3.0)).abs() < 1e-12);
    assert!((r[2].rrf - 1.0 / 62.0).abs() < 1e-12);
    assert!((r[3].rrf - 1.0 / 63.0).abs() < 1e-12);
    assert_eq!((r[2].keyword_rank, r[2].vector_rank), (None, Some(2)));
    assert_eq!(r[2].similarity, Some(0.8));
    assert_eq!((r[3].keyword_rank, r[3].vector_rank), (Some(3), None));
    assert_eq!(r[3].similarity, None);
}

#[test]
fn t_fus_01_weights_shift_order() {
    // Arrange: 키워드만 1위인 1, 벡터만 1위인 2
    let kw = [1];
    let vec = [vh(2, 0.5)];
    // Act
    let kw_heavy = fuse_rrf(
        &kw,
        &vec,
        &RrfParams {
            keyword_weight: 2.0,
            ..Default::default()
        },
    );
    let vec_heavy = fuse_rrf(
        &kw,
        &vec,
        &RrfParams {
            vector_weight: 2.0,
            ..Default::default()
        },
    );
    // Assert
    assert_eq!(ids(&kw_heavy), vec![1, 2]);
    assert_eq!(ids(&vec_heavy), vec![2, 1]);
}

// ---------------------------------------------------------------- T-fus-02 (Business rule)
#[test]
fn t_fus_02_raw_scores_do_not_affect_order() {
    // Arrange: 같은 순위, 코사인 점수만 극단적으로 다른 두 입력
    let kw = [5, 6, 7];
    let low = [vh(7, 0.01), vh(5, 0.005), vh(8, 0.001)];
    let high = [vh(7, 0.999), vh(5, 0.998), vh(8, 0.997)];
    // Act
    let a = fuse_rrf(&kw, &low, &RrfParams::default());
    let b = fuse_rrf(&kw, &high, &RrfParams::default());
    // Assert: 순서와 RRF 값이 같다 (점수는 similarity 표시 필드에만 남는다)
    assert_eq!(ids(&a), ids(&b));
    assert!(a.iter().zip(&b).all(|(x, y)| x.rrf == y.rrf));
}

#[test]
fn t_fus_02_ties_break_deterministically() {
    // 키워드 1위 100 과 벡터 1위 50 은 RRF 동점 -> 키워드 순위가 있는 쪽이 먼저
    let r = fuse_rrf(&[100], &[vh(50, 0.9)], &RrfParams::default());
    assert_eq!(ids(&r), vec![100, 50]);
    // 같은 입력은 항상 같은 결과
    let r1 = fuse_rrf(&[3, 1, 2], &[], &RrfParams::default());
    let r2 = fuse_rrf(&[3, 1, 2], &[], &RrfParams::default());
    assert_eq!(ids(&r1), vec![3, 1, 2]);
    assert_eq!(r1, r2);
    // 가중치 0 이면 모두 0점 -> 키워드 순위 -> 벡터 순위 순
    let z = fuse_rrf(
        &[9, 8],
        &[vh(7, 0.9)],
        &RrfParams {
            keyword_weight: 0.0,
            vector_weight: 0.0,
            ..Default::default()
        },
    );
    assert_eq!(ids(&z), vec![9, 8, 7]);
}

// ---------------------------------------------------------------- T-fus-03 (Boundary)
#[test]
fn t_fus_03_empty_and_one_sided_inputs() {
    let p = RrfParams::default();
    assert!(fuse_rrf(&[], &[], &p).is_empty());
    assert_eq!(ids(&fuse_rrf(&[4, 2], &[], &p)), vec![4, 2]);
    assert_eq!(
        ids(&fuse_rrf(&[], &[vh(8, 0.9), vh(6, 0.8)], &p)),
        vec![8, 6]
    );
}

#[test]
fn t_fus_03_duplicate_ids_use_first_occurrence_only() {
    // Arrange: 키워드 [1, 1, 2], 벡터 [3, 3, 1]
    let r = fuse_rrf(
        &[1, 1, 2],
        &[vh(3, 0.9), vh(3, 0.95), vh(1, 0.5)],
        &RrfParams::default(),
    );
    // Assert: 중복 제거 후 순위 — 1:(kw1, vec2), 2:(kw2), 3:(vec1)
    let by = |id| r.iter().find(|h| h.asset_id == id).unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!((by(1).keyword_rank, by(1).vector_rank), (Some(1), Some(2)));
    assert_eq!(by(2).keyword_rank, Some(2));
    assert_eq!(by(3).vector_rank, Some(1));
    assert_eq!(by(3).similarity, Some(0.9));
}

#[test]
fn t_fus_03_invalid_params_are_neutralised() {
    // Arrange: NaN k / 음수 가중치 — 패닉·NaN 점수 없이 처리
    let p = RrfParams {
        k: f32::NAN,
        keyword_weight: -1.0,
        vector_weight: 1.0,
    };
    // Act
    let r = fuse_rrf(&[1], &[vh(2, 0.5)], &p);
    // Assert
    assert!(r.iter().all(|h| h.rrf.is_finite() && h.rrf >= 0.0));
    assert_eq!(ids(&r), vec![2, 1]);
}

// ---------------------------------------------------------------- degraded (Business rule)
#[test]
fn t_fus_degraded_none_vector_gives_keyword_order_and_flag() {
    let p = RrfParams::default();
    let d = fuse_hybrid(&[7, 3, 9], None, &p);
    assert!(d.degraded);
    assert_eq!(ids(&d.hits), vec![7, 3, 9]);
    // 벡터 검색은 됐으나 후보 0 건이면 degraded 가 아니다
    let ok = fuse_hybrid(&[7, 3, 9], Some(&[]), &p);
    assert!(!ok.degraded);
    assert_eq!(ids(&ok.hits), vec![7, 3, 9]);
    // 벡터가 있으면 결합 결과와 같다
    let v = [vh(3, 0.9)];
    assert_eq!(
        fuse_hybrid(&[7, 3, 9], Some(&v), &p).hits,
        fuse_rrf(&[7, 3, 9], &v, &p)
    );
}
