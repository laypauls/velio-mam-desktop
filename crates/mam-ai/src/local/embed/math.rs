//! 임베딩 후처리 — 평균 풀링, L2 정규화, 유한성 검사.

use crate::error::{AiError, Result};

/// 제자리 L2 정규화. 영벡터·NaN/Inf 는 오류 (코사인 인덱스에 넣을 수 없다).
pub(crate) fn l2_normalize(v: &mut [f32]) -> Result<()> {
    let norm2: f32 = v.iter().map(|x| x * x).sum();
    if !norm2.is_finite() || norm2 <= f32::MIN_POSITIVE {
        return Err(AiError::ModelOutput(
            "embedding is zero or not finite".into(),
        ));
    }
    let inv = norm2.sqrt().recip();
    v.iter_mut().for_each(|x| *x *= inv);
    Ok(())
}

/// `hidden` 은 `[seq, dim]` 행 우선. `mask[i] == 1` 인 토큰만 평균 낸다 (sentence-transformers 평균 풀링과 동일).
pub(crate) fn mean_pool(hidden: &[f32], mask: &[i64], dim: usize) -> Vec<f32> {
    let mut out = vec![0f32; dim];
    let mut count = 0f32;
    for (row, m) in hidden.chunks_exact(dim).zip(mask) {
        if *m != 0 {
            out.iter_mut().zip(row).for_each(|(o, x)| *o += x);
            count += 1.0;
        }
    }
    if count > 0.0 {
        let inv = count.recip();
        out.iter_mut().for_each(|o| *o *= inv);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_pool_ignores_masked_tokens() {
        // Arrange: 3 토큰 x 2 차원, 마지막 토큰은 패딩
        let hidden = [1.0, 2.0, 3.0, 4.0, 100.0, 100.0];
        // Act
        let pooled = mean_pool(&hidden, &[1, 1, 0], 2);
        // Assert
        assert_eq!(pooled, vec![2.0, 3.0]);
    }

    #[test]
    fn l2_normalize_rejects_zero_vector() {
        let mut v = vec![0.0f32; 4];
        assert!(l2_normalize(&mut v).is_err());
        let mut v = vec![3.0f32, 4.0];
        l2_normalize(&mut v).unwrap();
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
    }
}
