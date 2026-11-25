//! `schedule` — 지정 요일(비우면 매일)·로컬 시각마다 시작. 앱이 꺼져 있어 슬롯을 여럿 놓쳐도 가장 최근 슬롯 1회만 따라잡는다.

use chrono::{Days, NaiveDate};
use serde_json::{Map, Value, json};

use super::PolicyError;
use super::local_time::{HourMinute, local_naive, resolve_local, weekday_index};
use super::trigger::{RescanTrigger, SlotCtx, TriggerSpec};

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

pub struct ScheduleTrigger;

#[derive(Debug)]
pub struct ScheduleSpec {
    /// 월 = 0 … 일 = 6, 정렬·중복 제거. 비어 있으면 매일
    days: Vec<u8>,
    at: HourMinute,
}

impl RescanTrigger for ScheduleTrigger {
    fn id(&self) -> &'static str {
        "schedule"
    }

    fn parse(&self, raw: &Map<String, Value>) -> Result<Box<dyn TriggerSpec>, PolicyError> {
        if raw.keys().any(|k| k != "kind" && k != "days" && k != "at") {
            return Err(PolicyError {
                field: "rescanPolicy.trigger",
            });
        }
        let bad_days = PolicyError {
            field: "rescanPolicy.trigger.days",
        };
        let mut days: Vec<u8> = Vec::new();
        match raw.get("days") {
            None => return Err(bad_days), // days 는 필수 ([] = 매일)
            Some(Value::Array(a)) => {
                for v in a {
                    let idx = v
                        .as_str()
                        .and_then(|s| DAYS.iter().position(|d| *d == s))
                        .ok_or(bad_days)?;
                    days.push(idx as u8);
                }
            }
            Some(_) => return Err(bad_days),
        }
        days.sort_unstable();
        days.dedup();
        if days.len() == 7 {
            days.clear(); // 모든 요일 = 매일
        }
        let at = raw
            .get("at")
            .and_then(Value::as_str)
            .and_then(HourMinute::parse)
            .ok_or(PolicyError {
                field: "rescanPolicy.trigger.at",
            })?;
        Ok(Box::new(ScheduleSpec { days, at }))
    }
}

impl ScheduleSpec {
    fn runs_on(&self, d: NaiveDate) -> bool {
        self.days.is_empty() || self.days.contains(&weekday_index(d))
    }
}

impl TriggerSpec for ScheduleSpec {
    fn kind(&self) -> &'static str {
        "schedule"
    }

    fn to_json(&self) -> Value {
        let days: Vec<&str> = self.days.iter().map(|i| DAYS[usize::from(*i)]).collect();
        json!({ "kind": "schedule", "days": days, "at": self.at.to_string() })
    }

    fn last_slot(&self, cx: &SlotCtx) -> Option<i64> {
        let d0 = local_naive(cx.now_ms, cx.zone).date();
        (0..=7u64)
            .filter_map(|k| d0.checked_sub_days(Days::new(k)))
            .filter(|d| self.runs_on(*d))
            .map(|d| resolve_local(d, self.at, cx.zone))
            .find(|s| *s <= cx.now_ms)
    }

    fn next_slot(&self, cx: &SlotCtx) -> Option<i64> {
        let d0 = local_naive(cx.now_ms, cx.zone).date();
        (0..=8u64)
            .filter_map(|k| d0.checked_add_days(Days::new(k)))
            .filter(|d| self.runs_on(*d))
            .map(|d| resolve_local(d, self.at, cx.zone))
            .find(|s| *s > cx.now_ms)
    }
}
