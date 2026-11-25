//! `interval` — 마지막 전체 스캔 완료 뒤 N시간이 지나면 시작 (옛 전역 `rescan_interval_hours` 와 같은 동작).
//! 슬롯은 `t + k·H` (k ≥ 1, t = 마지막 완료 시각)에 고정돼 `last_slot >= t ⇔ now - t >= H` 가 정확히 유지된다.

use serde_json::{Map, Value, json};

use super::trigger::{RescanTrigger, SlotCtx, TriggerSpec};
use super::{INTERVAL_MAX_HOURS, INTERVAL_MIN_HOURS, PolicyError};

const HOUR_MS: i64 = 3_600_000;

pub struct IntervalTrigger;

#[derive(Debug)]
pub struct IntervalSpec {
    pub hours: u32,
}

impl RescanTrigger for IntervalTrigger {
    fn id(&self) -> &'static str {
        "interval"
    }

    fn parse(&self, raw: &Map<String, Value>) -> Result<Box<dyn TriggerSpec>, PolicyError> {
        if raw.keys().any(|k| k != "kind" && k != "hours") {
            return Err(PolicyError { field: "rescanPolicy.trigger" });
        }
        let hours = raw
            .get("hours")
            .and_then(Value::as_i64)
            .filter(|h| (i64::from(INTERVAL_MIN_HOURS)..=i64::from(INTERVAL_MAX_HOURS)).contains(h))
            .and_then(|h| u32::try_from(h).ok())
            .ok_or(PolicyError { field: "rescanPolicy.trigger.hours" })?;
        Ok(Box::new(IntervalSpec { hours }))
    }
}

impl IntervalSpec {
    fn period_ms(&self) -> i64 {
        i64::from(self.hours) * HOUR_MS
    }
}

impl TriggerSpec for IntervalSpec {
    fn kind(&self) -> &'static str {
        "interval"
    }

    fn to_json(&self) -> Value {
        json!({ "kind": "interval", "hours": self.hours })
    }

    fn last_slot(&self, cx: &SlotCtx) -> Option<i64> {
        let h = self.period_ms();
        match cx.last_full_scan_at {
            None => Some(cx.now_ms),
            Some(t) if cx.now_ms - t >= h => Some(t + (cx.now_ms - t) / h * h),
            Some(_) => None,
        }
    }

    fn next_slot(&self, cx: &SlotCtx) -> Option<i64> {
        let h = self.period_ms();
        match cx.last_full_scan_at {
            None => Some(cx.now_ms + h),
            Some(t) => {
                let k = ((cx.now_ms - t).div_euclid(h) + 1).max(1);
                Some(t + k * h)
            }
        }
    }
}
