//! 로컬 시각 계산 — 시간대는 주입(`LocalZone`)해 시험에서 고정한다. DST 경계는 앞쪽 유효 시각으로 맞춘다.

use std::fmt;

use chrono::{Datelike, NaiveDate, NaiveDateTime};

const DAY_MS: i64 = 86_400_000;

/// UTC 시각 -> 로컬 UTC 오프셋 (초, 동쪽 +, DST 반영).
pub trait LocalZone: fmt::Debug + Send + Sync {
    fn offset_secs(&self, utc_ms: i64) -> i32;
}

/// OS 로컬 시간대 (chrono 규칙 — 알 수 없으면 0). 시간대 변경은 호출마다 반영된다.
#[derive(Debug)]
pub struct OsLocalZone;

impl LocalZone for OsLocalZone {
    fn offset_secs(&self, utc_ms: i64) -> i32 {
        use chrono::{Local, Offset, TimeZone};
        Local
            .timestamp_millis_opt(utc_ms)
            .single()
            .map_or(0, |dt| dt.offset().fix().local_minus_utc())
    }
}

/// 고정 오프셋 (초) — 시험·DST 없는 지역.
#[derive(Debug)]
pub struct FixedOffsetZone(pub i32);

impl LocalZone for FixedOffsetZone {
    fn offset_secs(&self, _utc_ms: i64) -> i32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HourMinute {
    pub hour: u8,
    pub minute: u8,
}

impl HourMinute {
    /// `^([01]\d|2[0-3]):[0-5]\d$`
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 5 || b[2] != b':' || !(b[0].is_ascii_digit() && b[1].is_ascii_digit() && b[3].is_ascii_digit() && b[4].is_ascii_digit()) {
            return None;
        }
        let hour = (b[0] - b'0') * 10 + (b[1] - b'0');
        let minute = (b[3] - b'0') * 10 + (b[4] - b'0');
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    pub fn minutes(&self) -> u16 {
        u16::from(self.hour) * 60 + u16::from(self.minute)
    }
}

impl fmt::Display for HourMinute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// UTC 시각의 로컬 벽시계 시각.
pub fn local_naive(utc_ms: i64, zone: &dyn LocalZone) -> NaiveDateTime {
    let local_ms = utc_ms.saturating_add(i64::from(zone.offset_secs(utc_ms)) * 1000);
    chrono::DateTime::from_timestamp_millis(local_ms).map_or_else(|| chrono::DateTime::UNIX_EPOCH.naive_utc(), |d| d.naive_utc())
}

/// 로컬 날짜·시각 -> UTC ms. 겹치는 시각(가을 전환)은 앞선 시점, 없는 시각(봄 전환)은 전환 시점.
/// 24시간 안에 전환이 최대 1번이라고 가정한다.
pub fn resolve_local(date: NaiveDate, at: HourMinute, zone: &dyn LocalZone) -> i64 {
    let wall = date
        .and_hms_opt(u32::from(at.hour), u32::from(at.minute), 0)
        .map_or(0, |d| d.and_utc().timestamp_millis());
    let off = |c: i64| i64::from(zone.offset_secs(c)) * 1000;
    let mut best: Option<i64> = None;
    for probe in [wall - DAY_MS, wall + DAY_MS] {
        let c = wall - off(probe);
        if c + off(c) == wall {
            best = Some(best.map_or(c, |b| b.min(c)));
        }
    }
    if let Some(c) = best {
        return c;
    }
    // 없는 시각: 벽시계가 `wall` 이상이 되는 가장 이른 UTC 시점 (= 전환 시점)
    let (mut lo, mut hi) = (wall - 2 * DAY_MS, wall + 2 * DAY_MS);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if mid + off(mid) >= wall {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// 요일 번호 (월 = 0 … 일 = 6)
pub fn weekday_index(date: NaiveDate) -> u8 {
    date.weekday().num_days_from_monday() as u8
}

#[cfg(test)]
pub(super) mod tests {
    //! 분류: Boundary (HH:MM) / Time (DST 겹침·없는 시각)
    use super::*;

    /// UTC 전환 시각별 오프셋 표 (시험용 DST 시간대)
    #[derive(Debug)]
    pub struct DstZone(pub Vec<(i64, i32)>, pub i32);

    impl LocalZone for DstZone {
        fn offset_secs(&self, utc_ms: i64) -> i32 {
            self.0
                .iter()
                .rev()
                .find(|(t, _)| *t <= utc_ms)
                .map_or(self.1, |(_, o)| *o)
        }
    }

    pub fn ms(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|d| d.and_hms_opt(h, mi, 0))
            .map(|d| d.and_utc().timestamp_millis())
            .unwrap()
    }

    #[test]
    fn t_rpol_hm_01_parse_accepts_only_zero_padded_24h() {
        assert_eq!(HourMinute::parse("03:00"), Some(HourMinute { hour: 3, minute: 0 }));
        assert_eq!(HourMinute::parse("23:59").unwrap().minutes(), 1439);
        assert_eq!(HourMinute::parse("00:00").unwrap().to_string(), "00:00");
        for bad in ["25:00", "3:00", "03:60", "", "03-00", "0300", "03:0a", "24:00", "03:000"] {
            assert_eq!(HourMinute::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn t_rpol_dst_01_fixed_zone_roundtrips() {
        let z = FixedOffsetZone(9 * 3600);
        let date = NaiveDate::from_ymd_opt(2026, 11, 21).unwrap();
        let at = HourMinute { hour: 3, minute: 0 };
        assert_eq!(resolve_local(date, at, &z), ms(2026, 11, 20, 18, 0));
        assert_eq!(local_naive(ms(2026, 11, 20, 18, 0), &z).to_string(), "2026-11-21 03:00:00");
    }

    #[test]
    fn t_rpol_dst_02_gap_resolves_to_the_transition_instant() {
        // 03-08 10:00 UTC 에 -5h -> -4h (로컬 02:00 -> 03:00): 02:30 은 없다
        let t = ms(2026, 3, 8, 7, 0);
        let z = DstZone(vec![(t, -4 * 3600)], -5 * 3600);
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();

        let got = resolve_local(date, HourMinute { hour: 2, minute: 30 }, &z);

        assert_eq!(got, t);
    }

    #[test]
    fn t_rpol_dst_03_overlap_resolves_to_the_earlier_instant() {
        // 11-01 06:00 UTC 에 -4h -> -5h (로컬 02:00 -> 01:00): 01:30 이 두 번 있다
        let t = ms(2026, 11, 1, 6, 0);
        let z = DstZone(vec![(t, -5 * 3600)], -4 * 3600);
        let date = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();

        let got = resolve_local(date, HourMinute { hour: 1, minute: 30 }, &z);

        assert_eq!(got, ms(2026, 11, 1, 5, 30)); // 첫 번째 01:30 (-4h)
    }
}
