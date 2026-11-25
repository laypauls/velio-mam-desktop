//! 지연 시간 통계.

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Summary {
    pub n: usize,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

/// 가장 가까운 순위(nearest-rank) 백분위.
fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

pub fn summarize(samples: &[Duration]) -> Summary {
    let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = if ms.is_empty() {
        0.0
    } else {
        ms.iter().sum::<f64>() / ms.len() as f64
    };
    Summary {
        n: ms.len(),
        p50_ms: pct(&ms, 50.0),
        p95_ms: pct(&ms, 95.0),
        p99_ms: pct(&ms, 99.0),
        max_ms: ms.last().copied().unwrap_or(0.0),
        mean_ms: mean,
    }
}

impl Summary {
    pub fn json(&self) -> String {
        format!(
            "{{\"n\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"mean_ms\":{:.3}}}",
            self.n, self.p50_ms, self.p95_ms, self.p99_ms, self.max_ms, self.mean_ms
        )
    }
}
