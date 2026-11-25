//! 벡터 인덱스 저장 적응형 제한 — 저장은 인덱스 전체를 다시 쓰므로(20만 건 ~3초, 200만 건 30초+),
//! 저장에 걸린 시간의 10배가 지난 뒤에만 다시 저장한다 (저장 시간 ≤ 벽시계의 약 10%).
//! 작은 라이브러리는 틱(5초)마다, 큰 라이브러리는 최대 10분 간격이다.

use std::time::{Duration, Instant};

/// 저장 간격 = 직전 저장 소요 × 이 배수.
const INTERVAL_FACTOR: u32 = 10;
/// 저장 간격 상한.
pub(super) const MAX_INTERVAL: Duration = Duration::from_secs(600);

/// 직전 저장 `(끝난 시각, 소요 시간)` 이 없으면 즉시 저장, 있으면 최소 간격이 지났을 때만 저장한다.
/// 시각은 호출자가 주입한다 (직접 시계를 읽지 않는다). `now` 가 `last` 보다 앞서면 아직 아니다.
pub(super) fn persist_due(last: Option<(Instant, Duration)>, now: Instant) -> bool {
    let Some((at, took)) = last else {
        return true;
    };
    let min = took.saturating_mul(INTERVAL_FACTOR).min(MAX_INTERVAL);
    now.saturating_duration_since(at) >= min && now >= at
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Time
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn t_persist_thr_01_no_history_is_due() {
        assert!(persist_due(None, Instant::now()));
    }

    #[test]
    fn t_persist_thr_02_within_interval_is_not_due() {
        let t0 = Instant::now();
        // 3초 저장 → 최소 30초
        assert!(!persist_due(Some((t0, secs(3))), t0 + secs(29)));
        assert!(!persist_due(Some((t0, secs(3))), t0));
    }

    #[test]
    fn t_persist_thr_03_exactly_at_boundary_is_due() {
        let t0 = Instant::now();
        assert!(persist_due(Some((t0, secs(3))), t0 + secs(30)));
        assert!(!persist_due(
            Some((t0, secs(3))),
            t0 + secs(30) - Duration::from_nanos(1)
        ));
    }

    #[test]
    fn t_persist_thr_04_interval_is_capped_at_ten_minutes() {
        let t0 = Instant::now();
        // 30초 저장은 300초 요구가 아니라 600초 상한 — 1시간짜리도 600초
        assert!(!persist_due(Some((t0, secs(3600))), t0 + secs(599)));
        assert!(persist_due(Some((t0, secs(3600))), t0 + secs(600)));
        assert!(persist_due(Some((t0, Duration::MAX)), t0 + secs(600)));
    }

    #[test]
    fn t_persist_thr_05_zero_duration_is_always_due() {
        let t0 = Instant::now();
        assert!(persist_due(Some((t0, Duration::ZERO)), t0));
    }

    #[test]
    fn t_persist_thr_06_clock_before_last_is_not_due() {
        let t0 = Instant::now() + secs(100);
        assert!(!persist_due(Some((t0, secs(1))), t0 - secs(50)));
    }
}
