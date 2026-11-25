//! 정책 판정 시험 (시계·시간대 주입). 분류: Happy / Boundary / Validation / Business rule / Time
//! 시간대: 고정 +09:00 (한국) — DST 는 주입 시간대(`DstZone`)로만 시험한다.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::local_time::tests::{DstZone, ms};
use super::local_time::{FixedOffsetZone, LocalZone};
use super::*;

static KST: FixedOffsetZone = FixedOffsetZone(9 * 3600);
const HOUR: i64 = 3_600_000;

/// 한국 시각 -> UTC ms
fn kst(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
    ms(y, m, d, h, mi) - 9 * HOUR
}

fn pol(v: Value) -> RescanPolicy {
    parse_policy(&v).expect("valid policy")
}

fn cx(now_ms: i64, last: Option<i64>) -> SlotCtx<'static> {
    SlotCtx {
        now_ms,
        zone: &KST,
        last_full_scan_at: last,
    }
}

#[test]
fn t_rpol_01_policy_is_normalized() {
    let p = pol(json!({
        "trigger": {"kind": "schedule", "days": ["sun", "mon", "mon"], "at": "03:00"},
        "onStart": true
    }));

    assert_eq!(
        p.to_json(),
        json!({
            "trigger": {"kind": "schedule", "days": ["mon", "sun"], "at": "03:00"},
            "window": null,
            "onStart": true
        })
    );
    let all = pol(
        json!({"trigger": {"kind": "schedule", "days": ["mon","tue","wed","thu","fri","sat","sun"], "at": "03:00"}}),
    );
    assert_eq!(
        all.to_json()["trigger"]["days"],
        json!([]),
        "all seven days collapse to daily"
    );
    let again = parse_policy(&p.to_json()).unwrap();
    assert_eq!(again.to_json(), p.to_json());
}

#[test]
fn t_rpol_02_shared_fixture_matches_the_validation_table() {
    // 정본: Rust·TS 공유 픽스처 (`field` = 오류 필드 경로, null = 유효)
    let text =
        include_str!("../../../tests/fixtures/rescan_policy_cases.json").replace("\r\n", "\n");
    let cases: Vec<Value> = serde_json::from_str(&text).unwrap();
    assert!(!cases.is_empty());
    for case in &cases {
        let got = parse_policy(&case["input"]);
        match case["field"].as_str() {
            None => assert!(got.is_ok(), "{} should be valid", case["input"]),
            Some(field) => assert_eq!(got.err().map(|e| e.field), Some(field), "{}", case["input"]),
        }
    }
}

#[test]
fn t_rpol_03_null_and_garbage_fall_back_to_the_global_default() {
    let (p, src) = effective_policy(None, 6);
    assert_eq!(
        (src, p.to_json()["trigger"].clone()),
        (
            PolicySource::Default,
            json!({"kind": "interval", "hours": 6})
        )
    );
    for garbage in ["not json", "[]", r#"{"trigger":{"kind":"weekly"}}"#, ""] {
        let (p, src) = effective_policy(Some(garbage), 24);
        assert_eq!(src, PolicySource::InvalidFallback, "{garbage}");
        assert_eq!(
            p.to_json()["trigger"],
            json!({"kind": "interval", "hours": 24})
        );
    }
    let (_, src) = effective_policy(Some(r#"{"trigger":{"kind":"manual"}}"#), 6);
    assert_eq!(src, PolicySource::Folder);
}

#[test]
fn t_rpol_10_default_policy_matches_the_old_interval_rule_exactly() {
    // 회귀 격자: 옛 규칙 `last.is_none_or(|t| now - t >= interval_ms)`
    for h in [1u32, 6, 720] {
        let p = RescanPolicy::default_interval(h);
        let interval = i64::from(h) * HOUR;
        let t = 1_700_000_000_000;
        let mut deltas: Vec<i64> = (-2 * HOUR..3 * interval).step_by(600_000).collect();
        deltas.extend([
            interval - 1,
            interval,
            interval + 1,
            2 * interval - 1,
            2 * interval,
        ]);
        for d in deltas {
            let c = cx(t + d, Some(t));
            assert_eq!(p.is_due(&c, false), d >= interval, "H={h} delta={d}");
        }
        assert!(p.is_due(&cx(t, None), false), "never scanned is always due");
    }
}

#[test]
fn t_rpol_11_slot_boundary_is_inclusive() {
    let p = pol(json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}}));
    let slot = kst(2026, 11, 21, 3, 0);
    let t = slot - HOUR; // 어제 슬롯 이후, 오늘 슬롯 이전에 완료

    assert!(p.is_due(&cx(slot, Some(t)), false));
    assert!(!p.is_due(&cx(slot - 1, Some(t)), false));
    // `>=` (spec C7): completion exactly at a slot still counts that slot as due — a scan completing after the slot is never re-run
    assert!(!p.is_due(&cx(slot + 1, Some(slot + 1)), false));
}

#[test]
fn t_rpol_12_three_missed_slots_catch_up_once() {
    let p = pol(json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}}));
    let t = kst(2026, 11, 17, 4, 0);
    let now = kst(2026, 11, 20, 9, 0); // 11-18·19·20 03:00 슬롯 3개를 놓침

    assert!(p.is_due(&cx(now, Some(t)), false));
    // 스캔이 끝나면(완료 시각 > 슬롯) 다음 슬롯까지 대기
    assert!(!p.is_due(&cx(now + 60_000, Some(now + 30_000)), false));
    assert!(p.is_due(&cx(kst(2026, 11, 21, 3, 0), Some(now + 30_000)), false));
}

#[test]
fn t_rpol_13_empty_days_means_daily_and_weekday_filter_applies() {
    let daily = pol(json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}}));
    let mon = pol(json!({"trigger": {"kind": "schedule", "days": ["mon"], "at": "03:00"}}));
    let sat_noon = kst(2026, 11, 21, 12, 0); // 2026-11-21 은 토요일

    assert_eq!(
        daily.trigger.last_slot(&cx(sat_noon, None)),
        Some(kst(2026, 11, 21, 3, 0))
    );
    assert_eq!(
        mon.trigger.last_slot(&cx(sat_noon, None)),
        Some(kst(2026, 11, 16, 3, 0))
    );
    assert_eq!(
        mon.trigger.next_slot(&cx(sat_noon, None)),
        Some(kst(2026, 11, 23, 3, 0))
    );
    let t = kst(2026, 11, 20, 12, 0);
    assert!(daily.is_due(&cx(sat_noon, Some(t)), false));
    assert!(
        !mon.is_due(&cx(sat_noon, Some(t)), false),
        "no Monday slot since the last scan"
    );
}

#[test]
fn t_rpol_14_window_across_midnight() {
    let w = TimeWindow {
        start: HourMinute::parse("22:00").unwrap(),
        end: HourMinute::parse("06:00").unwrap(),
    };
    let at = |h, m| kst(2026, 11, 21, h, m);

    assert!(w.contains(at(23, 0), &KST));
    assert!(w.contains(at(5, 59), &KST));
    assert!(!w.contains(at(6, 0), &KST));
    assert!(!w.contains(at(21, 59), &KST));
    assert!(w.contains(at(22, 0), &KST));
    let day = TimeWindow {
        start: HourMinute::parse("09:00").unwrap(),
        end: HourMinute::parse("18:00").unwrap(),
    };
    assert!(day.contains(at(9, 0), &KST) && !day.contains(at(18, 0), &KST));
}

fn slot_set(spec: &dyn TriggerSpec, zone: &dyn LocalZone, from: i64, to: i64) -> Vec<i64> {
    let mut slots = BTreeSet::new();
    let mut now = from;
    while now <= to {
        let c = SlotCtx {
            now_ms: now,
            zone,
            last_full_scan_at: None,
        };
        slots.extend(spec.last_slot(&c));
        now += 5 * 60_000;
    }
    slots.into_iter().collect()
}

#[test]
fn t_rpol_15_dst_gap_and_overlap_give_one_slot_per_day() {
    // 봄(없는 02:30): 전환 시점으로 맞춘다 / 가을(겹치는 01:30): 앞선 시점 하나만
    let spring = DstZone(vec![(ms(2026, 3, 8, 7, 0), -4 * 3600)], -5 * 3600);
    let fall = DstZone(vec![(ms(2026, 11, 1, 6, 0), -5 * 3600)], -4 * 3600);
    let at = |hm: &str| pol(json!({"trigger": {"kind": "schedule", "days": [], "at": hm}}));

    let s = slot_set(
        &*at("02:30").trigger,
        &spring,
        ms(2026, 3, 6, 12, 0),
        ms(2026, 3, 11, 12, 0),
    );
    let f = slot_set(
        &*at("01:30").trigger,
        &fall,
        ms(2026, 10, 30, 12, 0),
        ms(2026, 11, 3, 12, 0),
    );

    assert_eq!(s.len(), 6);
    assert!(
        s.contains(&ms(2026, 3, 8, 7, 0)),
        "the missing wall time maps to the transition instant"
    );
    assert_eq!(f.len(), 5);
    assert!(f.contains(&ms(2026, 11, 1, 5, 30)) && !f.contains(&ms(2026, 11, 1, 6, 30)));
    for w in s.windows(2).chain(f.windows(2)) {
        let gap = w[1] - w[0];
        assert!((22 * HOUR..=26 * HOUR).contains(&gap), "gap {gap}");
    }
}

#[test]
fn t_rpol_16_clock_going_backwards_never_triggers() {
    let t = kst(2026, 11, 21, 12, 0);
    let now = t - 5 * HOUR;
    for p in [
        pol(json!({"trigger": {"kind": "interval", "hours": 1}})),
        pol(json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}})),
    ] {
        assert!(!p.is_due(&cx(now, Some(t)), false));
    }
}

#[test]
fn t_rpol_17_manual_is_never_due_except_for_never_scanned_folders() {
    let p = pol(json!({"trigger": {"kind": "manual"}}));
    let c = cx(kst(2026, 11, 21, 12, 0), Some(kst(2026, 1, 1, 0, 0)));

    assert!(!p.is_due(&c, false));
    assert_eq!(p.next_rescan_at(&c), None);
    assert!(p.is_due(&cx(c.now_ms, None), false));
}

#[test]
fn t_rpol_18_next_rescan_moves_to_the_window_start() {
    let p = pol(json!({
        "trigger": {"kind": "interval", "hours": 6},
        "window": {"start": "22:00", "end": "06:00"}
    }));
    let t = kst(2026, 11, 21, 6, 0);
    let now = kst(2026, 11, 21, 7, 0);

    // 슬롯 12:00 은 시간대 밖 -> 22:00
    assert_eq!(
        p.next_rescan_at(&cx(now, Some(t))),
        Some(kst(2026, 11, 21, 22, 0))
    );
    // 슬롯 23:00(t = 17:00) 은 시간대 안 -> 그대로
    let t2 = kst(2026, 11, 21, 17, 0);
    assert_eq!(
        p.next_rescan_at(&cx(now, Some(t2))),
        Some(kst(2026, 11, 21, 23, 0))
    );
    // 이미 실행 대상이고 시간대 밖이면 다음 시작
    let due = cx(kst(2026, 11, 21, 15, 0), Some(t));
    assert!(!p.is_due(&due, false));
    assert_eq!(p.next_rescan_at(&due), Some(kst(2026, 11, 21, 22, 0)));
}

#[test]
fn t_rpol_19_never_scanned_is_due_regardless_of_policy_and_window() {
    let p =
        pol(json!({"trigger": {"kind": "manual"}, "window": {"start": "22:00", "end": "06:00"}}));
    let c = cx(kst(2026, 11, 21, 12, 0), None);

    assert!(p.is_due(&c, false));
    assert_eq!(p.next_rescan_at(&c), Some(c.now_ms));
}

#[test]
fn t_rpol_20_on_start_fires_on_first_pass_only_and_ignores_the_window() {
    let p = pol(json!({
        "trigger": {"kind": "manual"},
        "window": {"start": "22:00", "end": "06:00"},
        "onStart": true
    }));
    let c = cx(kst(2026, 11, 21, 12, 0), Some(kst(2026, 11, 21, 11, 0)));

    assert!(p.is_due(&c, true));
    assert!(!p.is_due(&c, false));
}

#[test]
fn t_rpol_21_window_gates_the_trigger() {
    let p = pol(json!({
        "trigger": {"kind": "interval", "hours": 1},
        "window": {"start": "22:00", "end": "06:00"}
    }));
    let t = kst(2026, 11, 21, 8, 0);

    assert!(
        !p.is_due(&cx(kst(2026, 11, 21, 12, 0), Some(t)), false),
        "due by interval but outside the window"
    );
    assert!(p.is_due(&cx(kst(2026, 11, 21, 23, 0), Some(t)), false));
}

#[test]
fn t_rpol_22_next_rescan_with_the_clock_behind_shows_a_slot_that_can_fire() {
    // 완료 시각 t = 11-21 12:00, 시계는 11-21 01:00 (뒤로 감) — 03:00 슬롯은 t 이전이라 실행되지 않는다
    let t = kst(2026, 11, 21, 12, 0);
    let now = kst(2026, 11, 21, 1, 0);
    let daily = pol(json!({"trigger": {"kind": "schedule", "days": [], "at": "03:00"}}));
    let interval = pol(json!({"trigger": {"kind": "interval", "hours": 6}}));

    assert_eq!(
        daily.next_rescan_at(&cx(now, Some(t))),
        Some(kst(2026, 11, 22, 3, 0))
    );
    assert_eq!(
        interval.next_rescan_at(&cx(now, Some(t))),
        Some(t + 6 * HOUR)
    );
}
