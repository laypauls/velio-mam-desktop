//! 시간대 수식 — 이 시간대 안에서만 재스캔을 **새로 시작**한다 (진행 중인 스캔은 멈추지 않는다, 요청서 지적 I).

use chrono::{Days, NaiveDate};

use super::local_time::{HourMinute, LocalZone, local_naive, resolve_local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeWindow {
    pub start: HourMinute,
    pub end: HourMinute,
}

impl TimeWindow {
    /// `start < end`: `[start, end)`. `start > end`: 자정을 넘는 `[start, 24:00) ∪ [00:00, end)`. (`start == end` 는 파싱에서 거부)
    pub fn contains(&self, now_ms: i64, zone: &dyn LocalZone) -> bool {
        use chrono::Timelike;
        let t = local_naive(now_ms, zone).time();
        let m = (t.hour() * 60 + t.minute()) as u16;
        let (s, e) = (self.start.minutes(), self.end.minutes());
        if s < e {
            s <= m && m < e
        } else {
            m >= s || m < e
        }
    }

    /// `from_ms` 이후 처음으로 시간대가 열려 있는 시각 (이미 안이면 그대로).
    pub fn next_open(&self, from_ms: i64, zone: &dyn LocalZone) -> i64 {
        if self.contains(from_ms, zone) {
            return from_ms;
        }
        let d0: NaiveDate = local_naive(from_ms, zone).date();
        (0..=1u64)
            .filter_map(|k| d0.checked_add_days(Days::new(k)))
            .map(|d| resolve_local(d, self.start, zone))
            .find(|s| *s >= from_ms)
            .unwrap_or(from_ms)
    }
}
