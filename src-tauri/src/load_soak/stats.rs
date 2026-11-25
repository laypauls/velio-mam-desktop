//! 백분위·선형 회귀 — 측정 보고용 (P50/P95/P99 는 최근접 순위, `.claude/rules/tests.md`).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

/// 정렬된 표본의 `p` 백분위 (0~100). 비면 0.
pub fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

pub fn summarize(samples: &[f64]) -> Summary {
    let mut v = samples.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Summary {
        n: v.len(),
        p50: pct(&v, 50.0),
        p95: pct(&v, 95.0),
        p99: pct(&v, 99.0),
        max: v.last().copied().unwrap_or(0.0),
    }
}

/// `y = a + b x` (x = 0, 1, 2 ...) 최소제곱 기울기와 결정계수. 표본이 2개 미만이면 (0, 0).
pub fn linreg(ys: &[f64]) -> (f64, f64) {
    let n = ys.len() as f64;
    if ys.len() < 2 {
        return (0.0, 0.0);
    }
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = ys.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (i, y) in ys.iter().enumerate() {
        let dx = i as f64 - mean_x;
        let dy = y - mean_y;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    let slope = sxy / sxx;
    let r2 = if syy == 0.0 {
        0.0
    } else {
        sxy * sxy / (sxx * syy)
    };
    (slope, r2)
}

pub fn mean(ys: &[f64]) -> f64 {
    if ys.is_empty() {
        0.0
    } else {
        ys.iter().sum::<f64>() / ys.len() as f64
    }
}

/// 고정 시드 선형 합동 난수 — 질의 집합·합성 데이터가 실행마다 같도록.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn raw(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.raw() as usize) % n.max(1)
    }

    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }

    pub fn unit(&mut self) -> f32 {
        (self.raw() % 1_000_000) as f32 / 1_000_000.0
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary
    use super::*;

    #[test]
    fn t_p4_stats_01_percentiles_and_regression() {
        // Arrange
        let xs: Vec<f64> = (1..=100).map(f64::from).collect();
        // Act
        let s = summarize(&xs);
        let (slope, r2) = linreg(&[1.0, 3.0, 5.0, 7.0]);
        // Assert
        assert_eq!(s.n, 100);
        assert_eq!(s.p50, 51.0);
        assert_eq!(s.p95, 95.0);
        assert_eq!(s.max, 100.0);
        assert!((slope - 2.0).abs() < 1e-9 && (r2 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn t_p4_stats_02_empty_and_single_samples_are_zero() {
        assert_eq!(summarize(&[]), Summary::default());
        assert_eq!(linreg(&[5.0]), (0.0, 0.0));
        assert_eq!(linreg(&[2.0, 2.0, 2.0]), (0.0, 0.0));
    }
}
