//! 폴더별 재스캔 일정 정책 — 트리거 1개(`interval`·`schedule`·`manual`) + 수식(`window`·`on_start`).
//!
//! 판정은 새 저장 상태 없이 `last_full_scan_at`(완료 시각)과 현재 시각만 쓴다. 정책은 "언제 시작하나"만 정한다 —
//! 재스캔 엔진은 하나다. 저장 형태 = IPC 형태 (camelCase JSON, 저장 시 정규화).

mod interval;
pub mod local_time;
mod manual;
mod schedule;
mod trigger;
mod window;

#[cfg(test)]
mod contract;
#[cfg(test)]
mod policy_tests;

use std::sync::LazyLock;

use serde_json::{Value, json};

pub use local_time::{FixedOffsetZone, HourMinute, LocalZone, OsLocalZone};
pub use trigger::{PolicyRegistry, RescanTrigger, SlotCtx, TriggerSpec};
pub use window::TimeWindow;

/// `interval` 하한 — 잠정값 (요청서 J: 전체 재스캔 소요 측정 후 조정). 상한은 전역 설정(`settings.rs`)과 같다.
pub const INTERVAL_MIN_HOURS: u32 = 1;
pub const INTERVAL_MAX_HOURS: u32 = 720;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid rescan policy: {field}")]
pub struct PolicyError {
    /// 필드 경로 (`rescanPolicy.trigger.hours` 등) — `IngestError::InvalidInput` 으로 전달된다
    pub field: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicySource {
    /// 폴더에 저장된 정책
    Folder,
    /// 저장된 정책 없음(NULL) — 전역 `rescan_interval_hours`
    Default,
    /// 저장된 값을 해석할 수 없어 전역 기본으로 동작
    InvalidFallback,
}

#[derive(Debug)]
pub struct RescanPolicy {
    pub trigger: Box<dyn TriggerSpec>,
    pub window: Option<TimeWindow>,
    pub on_start: bool,
}

pub fn policy_registry() -> &'static PolicyRegistry {
    static REGISTRY: LazyLock<PolicyRegistry> = LazyLock::new(PolicyRegistry::builtin);
    &REGISTRY
}

fn err(field: &'static str) -> PolicyError {
    PolicyError { field }
}

fn parse_window(raw: &Value) -> Result<Option<TimeWindow>, PolicyError> {
    let obj = match raw {
        Value::Null => return Ok(None),
        Value::Object(o) => o,
        _ => return Err(err("rescanPolicy.window")),
    };
    if obj.keys().any(|k| k != "start" && k != "end") {
        return Err(err("rescanPolicy.window"));
    }
    let time = |key: &str, field| {
        obj.get(key)
            .and_then(Value::as_str)
            .and_then(HourMinute::parse)
            .ok_or(err(field))
    };
    let start = time("start", "rescanPolicy.window.start")?;
    let end = time("end", "rescanPolicy.window.end")?;
    if start == end {
        return Err(err("rescanPolicy.window"));
    }
    Ok(Some(TimeWindow { start, end }))
}

/// IPC·저장 JSON 을 검증해 정책으로 만든다. 오류는 첫 번째 잘못된 필드 경로를 돌려준다.
pub fn parse_policy(raw: &Value) -> Result<RescanPolicy, PolicyError> {
    let Value::Object(obj) = raw else {
        return Err(err("rescanPolicy"));
    };
    if obj
        .keys()
        .any(|k| !matches!(k.as_str(), "trigger" | "window" | "onStart"))
    {
        return Err(err("rescanPolicy"));
    }
    let Some(Value::Object(trigger)) = obj.get("trigger") else {
        return Err(err("rescanPolicy.trigger"));
    };
    let kind = trigger
        .get("kind")
        .and_then(Value::as_str)
        .and_then(|k| policy_registry().get(k))
        .ok_or(err("rescanPolicy.trigger.kind"))?;
    let trigger = kind.parse(trigger)?;
    let window = obj.get("window").map_or(Ok(None), parse_window)?;
    let on_start = match obj.get("onStart") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(err("rescanPolicy.onStart")),
    };
    Ok(RescanPolicy {
        trigger,
        window,
        on_start,
    })
}

impl RescanPolicy {
    /// 폴더 값이 없을 때의 정책 — 전역 `rescan_interval_hours` 를 `interval` 로 해석한다.
    pub fn default_interval(hours: u32) -> Self {
        let trigger = Box::new(interval::IntervalSpec {
            hours: hours.clamp(INTERVAL_MIN_HOURS, INTERVAL_MAX_HOURS),
        });
        Self {
            trigger,
            window: None,
            on_start: false,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "trigger": self.trigger.to_json(),
            "window": self.window.map(|w| json!({ "start": w.start.to_string(), "end": w.end.to_string() })),
            "onStart": self.on_start,
        })
    }

    /// 지금 재스캔을 시작해야 하는가 (이미 대기·실행 중인지는 호출자가 본다).
    /// 한 번도 끝난 스캔이 없으면 정책·시간대와 무관하게 true, `on_start` 는 첫 점검에서 시간대를 무시하고 true.
    pub fn is_due(&self, cx: &SlotCtx, first_pass: bool) -> bool {
        let Some(t) = cx.last_full_scan_at else {
            return true;
        };
        if self.on_start && first_pass {
            return true;
        }
        let by_trigger = self.trigger.last_slot(cx).is_some_and(|s| s >= t);
        by_trigger && self.window.is_none_or(|w| w.contains(cx.now_ms, cx.zone))
    }

    /// 다음 자동 재스캔 예정 시각 (표시용). 자동 예정이 없으면(`manual`) None.
    pub fn next_rescan_at(&self, cx: &SlotCtx) -> Option<i64> {
        let t = cx.last_full_scan_at;
        let last = self.trigger.last_slot(cx);
        // 시계가 마지막 완료 시각보다 뒤로 가 있으면 t 이후의 첫 슬롯을 보인다 (t 이전 슬롯은 실행되지 않는다)
        let ahead = SlotCtx {
            now_ms: t.map_or(cx.now_ms, |t| cx.now_ms.max(t)),
            zone: cx.zone,
            last_full_scan_at: t,
        };
        let next = self.trigger.next_slot(&ahead);
        if t.is_some() && last.is_none() && next.is_none() {
            return None;
        }
        let due_now = match t {
            None => true,
            Some(t) => last.is_some_and(|s| s >= t),
        };
        let cand = if due_now { cx.now_ms } else { next? };
        Some(match (t, self.window) {
            (Some(_), Some(w)) => w.next_open(cand, cx.zone),
            _ => cand,
        })
    }
}

/// 저장된 정책 텍스트의 유효 정책. NULL = 전역 기본, 해석 불가(JSON 아님·검증 실패) = 전역 기본 + `InvalidFallback`.
pub fn effective_policy(stored: Option<&str>, default_hours: u32) -> (RescanPolicy, PolicySource) {
    let Some(text) = stored else {
        return (
            RescanPolicy::default_interval(default_hours),
            PolicySource::Default,
        );
    };
    match serde_json::from_str::<Value>(text)
        .map_err(|_| err("rescanPolicy"))
        .and_then(|v| parse_policy(&v))
    {
        Ok(p) => (p, PolicySource::Folder),
        Err(_) => (
            RescanPolicy::default_interval(default_hours),
            PolicySource::InvalidFallback,
        ),
    }
}
