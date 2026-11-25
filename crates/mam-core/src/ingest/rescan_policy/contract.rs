//! 트리거 공통 계약 시험 — 새 트리거는 `trigger_contract_tests!` 한 블록을 붙인다.
//! 분류: Happy (슬롯 경계·단조성·왕복) / Validation (잘못된 옵션 -> 기대 필드 경로) / Time (고정 오프셋·DST 시간대)

use serde_json::{Map, Value, json};

use super::local_time::tests::{DstZone, ms};
use super::local_time::{FixedOffsetZone, LocalZone};
use super::policy_registry;
use super::trigger::{SlotCtx, TriggerSpec};

/// 출하된 트리거 kind — 새 트리거를 추가하면 여기에도 적는다 (kind 는 배포 후 rename 금지).
const SHIPPED_TRIGGER_KINDS: &[&str] = &["interval", "schedule", "manual"];

fn parse(v: &Value) -> Result<Box<dyn TriggerSpec>, super::PolicyError> {
    let obj: &Map<String, Value> = v.as_object().expect("object");
    let kind = obj.get("kind").and_then(Value::as_str).expect("kind");
    policy_registry().get(kind).expect("registered").parse(obj)
}

fn zones() -> Vec<Box<dyn LocalZone>> {
    let dst = DstZone(vec![(ms(2026, 3, 8, 7, 0), -4 * 3600)], -5 * 3600);
    vec![Box::new(FixedOffsetZone(9 * 3600)), Box::new(dst)]
}

pub(super) fn check_kind(kind: &str, valid: &[Value]) {
    assert!(
        SHIPPED_TRIGGER_KINDS.contains(&kind),
        "{kind} must be listed in SHIPPED_TRIGGER_KINDS"
    );
    assert!(policy_registry().get(kind).is_some());
    for v in valid {
        assert_eq!(parse(v).expect("valid sample").kind(), kind);
    }
}

pub(super) fn check_slot_bounds(valid: &[Value]) {
    let base = ms(2026, 3, 6, 0, 0);
    let step = 9 * 86_400_000 / 200;
    for v in valid {
        let spec = parse(v).expect("valid sample");
        for zone in zones() {
            for t in [None, Some(base)] {
                for i in 0..200 {
                    let now = base + i * step;
                    let cx = SlotCtx {
                        now_ms: now,
                        zone: &*zone,
                        last_full_scan_at: t,
                    };
                    if let Some(l) = spec.last_slot(&cx) {
                        assert!(l <= now, "{v}: last_slot {l} > now {now}");
                    }
                    if let Some(n) = spec.next_slot(&cx) {
                        assert!(n > now, "{v}: next_slot {n} <= now {now}");
                    }
                }
            }
        }
    }
}

pub(super) fn check_monotonic(valid: &[Value]) {
    let base = ms(2026, 3, 6, 0, 0);
    let step = 9 * 86_400_000 / 200;
    for v in valid {
        let spec = parse(v).expect("valid sample");
        for zone in zones() {
            let mut prev: Option<i64> = None;
            for i in 0..200 {
                let cx = SlotCtx {
                    now_ms: base + i * step,
                    zone: &*zone,
                    last_full_scan_at: Some(base),
                };
                if let Some(l) = spec.last_slot(&cx) {
                    assert!(prev.is_none_or(|p| l >= p), "{v}: last_slot went backwards");
                    prev = Some(l);
                }
            }
        }
    }
}

pub(super) fn check_invalid(invalid: &[(Value, &str)]) {
    for (v, field) in invalid {
        let err = parse(v)
            .err()
            .unwrap_or_else(|| panic!("{v} must be rejected"));
        assert_eq!(err.field, *field, "{v}");
    }
}

pub(super) fn check_roundtrip(valid: &[Value]) {
    for v in valid {
        let spec = parse(v).expect("valid sample");
        let json = spec.to_json();
        let again = parse(&json).expect("normalized form parses");
        assert_eq!(again.to_json(), json, "{v}");
    }
}

macro_rules! trigger_contract_tests {
    ($module:ident, $kind:literal, valid = [$($valid:expr),+ $(,)?], invalid = [$(($bad:expr, $field:literal)),* $(,)?]) => {
        mod $module {
            use super::*;

            fn valid() -> Vec<Value> {
                vec![$($valid),+]
            }

            #[test]
            fn t_rtrig_01_kind_is_stable_and_shipped() {
                check_kind($kind, &valid());
            }

            #[test]
            fn t_rtrig_02_slots_bracket_now() {
                check_slot_bounds(&valid());
            }

            #[test]
            fn t_rtrig_03_last_slot_is_monotonic() {
                check_monotonic(&valid());
            }

            #[test]
            fn t_rtrig_04_invalid_options_name_the_field() {
                check_invalid(&[$(($bad, $field)),*]);
            }

            #[test]
            fn t_rtrig_05_normalized_json_roundtrips() {
                check_roundtrip(&valid());
            }
        }
    };
}

trigger_contract_tests!(
    interval,
    "interval",
    valid = [
        json!({"kind": "interval", "hours": 1}),
        json!({"kind": "interval", "hours": 6}),
        json!({"kind": "interval", "hours": 720})
    ],
    invalid = [
        (json!({"kind": "interval"}), "rescanPolicy.trigger.hours"),
        (
            json!({"kind": "interval", "hours": 0}),
            "rescanPolicy.trigger.hours"
        ),
        (
            json!({"kind": "interval", "hours": 721}),
            "rescanPolicy.trigger.hours"
        ),
        (
            json!({"kind": "interval", "hours": 1.5}),
            "rescanPolicy.trigger.hours"
        ),
        (
            json!({"kind": "interval", "hours": 6, "x": 1}),
            "rescanPolicy.trigger"
        ),
    ]
);

trigger_contract_tests!(
    schedule,
    "schedule",
    valid = [
        json!({"kind": "schedule", "days": [], "at": "03:00"}),
        json!({"kind": "schedule", "days": ["fri", "mon", "mon"], "at": "23:59"}),
        json!({"kind": "schedule", "days": [], "at": "02:30"}),
    ],
    invalid = [
        (
            json!({"kind": "schedule", "days": ["monday"], "at": "03:00"}),
            "rescanPolicy.trigger.days"
        ),
        (
            json!({"kind": "schedule", "days": "mon", "at": "03:00"}),
            "rescanPolicy.trigger.days"
        ),
        (
            json!({"kind": "schedule", "days": [], "at": "25:00"}),
            "rescanPolicy.trigger.at"
        ),
        (
            json!({"kind": "schedule", "days": []}),
            "rescanPolicy.trigger.at"
        ),
        (
            json!({"kind": "schedule", "at": "03:00"}),
            "rescanPolicy.trigger.days"
        ),
        (
            json!({"kind": "schedule", "at": "03:00"}),
            "rescanPolicy.trigger.days"
        ),
        (
            json!({"kind": "schedule", "at": "03:00", "x": 1}),
            "rescanPolicy.trigger"
        ),
    ]
);

trigger_contract_tests!(
    manual,
    "manual",
    valid = [json!({"kind": "manual"})],
    invalid = [(
        json!({"kind": "manual", "hours": 1}),
        "rescanPolicy.trigger"
    )]
);
