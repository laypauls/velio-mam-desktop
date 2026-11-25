//! 구간 집계 (목차 레일·행 모델용) — 현재 질의·필터에 맞는 문서를 정렬 순서대로 구간별로 센다.
//!
//! - 촬영일·수정일 정렬: **로컬 시간대의 월**(`"YYYY-MM"`) + 값 없는 문서(`"none"`, 방향과 무관하게 맨 뒤).
//!   프론트 월 머리글(`monthKey`)과 같은 구간이다. 시간대는 호출자가 `utc_offset` 로 주입한다 (제품: OS 로컬).
//! - 이름·제목 정렬: 첫 글자 구간. 이름 키(NFC + 소문자 UTF-8 앞 8바이트 = `*_hi`)의 **바이트 순서 = 코드포인트 순서**
//!   라서 구간이 정렬 순서에서 연속이다 ([`name_initial_bounds`]).
//! - 그 밖의 정렬(크기·길이·관련도)은 구간이 없다 (`SectionKind::None`, 단일 구간 `"all"`).
//!
//! 개수가 0인 구간은 돌려주지 않는다. Σ`count` = `total`. 방향이 내림차순이면 구간 순서도 뒤집힌다.

use std::sync::Arc;

use tantivy::collector::{Collector, SegmentCollector};
use tantivy::columnar::Column;
use tantivy::{DocId, Score, SegmentReader};

use super::{CoordRequest, SearchHandle, SortField};
use crate::error::{Result, SearchError};
use crate::search::{SearchRequest, Sort, build_query};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    Month,
    Initial,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// `"YYYY-MM"` / `"none"`(날짜 없음) / 이름 구간 키 (`"sym"`, `"a"`..`"z"`, `"etc"`, `"ㄱ"`..`"ㅎ"`, `"etc_end"`) / `"all"`
    pub key: String,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionIndex {
    pub total: u64,
    pub generation: u64,
    pub kind: SectionKind,
    /// 정렬 순서대로.
    pub sections: Vec<Section>,
}

/// OS 로컬 시간대의 UTC 오프셋(초, 동쪽 +). 해당 시각의 규칙(DST 포함)을 따르고, 알 수 없으면 0.
pub fn local_utc_offset_secs(utc_secs: i64) -> i32 {
    use chrono::{Local, Offset, TimeZone};
    Local
        .timestamp_opt(utc_secs, 0)
        .single()
        .map_or(0, |dt| dt.offset().fix().local_minus_utc())
}

// ---- 이름 구간 ----

fn utf8_hi(cp: u32) -> u64 {
    let mut buf = [0u8; 8];
    if let Some(c) = char::from_u32(cp) {
        c.encode_utf8(&mut buf);
    }
    u64::from_be_bytes(buf)
}

/// 한글 음절의 초성 구간 시작 (초성 인덱스, 라벨). ㄲ·ㄸ·ㅃ·ㅆ·ㅉ 는 앞 자음 구간에 합친다.
const CHOSEONG: [(u32, &str); 14] = [
    (0, "ㄱ"),
    (2, "ㄴ"),
    (3, "ㄷ"),
    (5, "ㄹ"),
    (6, "ㅁ"),
    (7, "ㅂ"),
    (9, "ㅅ"),
    (11, "ㅇ"),
    (12, "ㅈ"),
    (14, "ㅊ"),
    (15, "ㅋ"),
    (16, "ㅌ"),
    (17, "ㅍ"),
    (18, "ㅎ"),
];
/// 음절 하나당 중성 21 × 종성 28
const SYLLABLES_PER_CHOSEONG: u32 = 588;
const HANGUL_BASE: u32 = 0xAC00;
/// 마지막 음절 `힣`(U+D7A3) 다음 코드포인트
const AFTER_HANGUL: u32 = 0xD7A4;

/// 이름 구간 `(키, 시작 hi 값)` — 정렬 순서(오름차순). 첫 구간 `"sym"` 은 0부터.
///
/// `sym`: `"a"` 앞 전부(숫자·기호·빈 이름) / `a`..`z` / `etc`: `z` 뒤 ~ `가` 앞 전부(악센트 라틴·가나·한자·한글 자모 낱자 포함) /
/// `ㄱ`..`ㅎ`: 한글 음절 초성별 / `etc_end`: `힣` 뒤 전부(호환 영역·이모지 등).
/// 정렬이 코드포인트 순이라 구간이 연속이어야 하므로 위 정의는 정렬 규칙을 따른 것이다.
pub fn name_initial_bounds() -> Vec<(String, u64)> {
    let mut v = vec![("sym".to_string(), 0u64)];
    for c in b'a'..=b'z' {
        v.push(((c as char).to_string(), utf8_hi(u32::from(c))));
    }
    v.push(("etc".into(), utf8_hi(u32::from(b'z') + 1)));
    for (idx, label) in CHOSEONG {
        v.push((
            label.to_string(),
            utf8_hi(HANGUL_BASE + idx * SYLLABLES_PER_CHOSEONG),
        ));
    }
    v.push(("etc_end".into(), utf8_hi(AFTER_HANGUL)));
    v
}

// ---- 월 구간 ----

/// 1970-01-01 기준 일 수 -> (연, 월). Howard Hinnant 의 civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m as u32)
}

/// (연, 월, 1일) -> 1970-01-01 기준 일 수.
fn days_from_civil(y: i64, m: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 달력 범위 제한 (연 1..=9999) — 엉뚱한 값이 수만 개 구간을 만들지 않게 한다.
const MIN_SECS: i64 = -62_135_596_800;
const MAX_SECS: i64 = 253_402_300_799;

/// `[min_secs, max_secs]` 를 덮는 월 구간 `(키, 시작 UTC 초)` — 오름차순. 월 시작은 그 시각의 로컬 오프셋으로 맞춘다.
fn month_bounds(
    min_secs: i64,
    max_secs: i64,
    utc_offset: &dyn Fn(i64) -> i32,
) -> Vec<(String, i64)> {
    let (lo, hi) = (
        min_secs.clamp(MIN_SECS, MAX_SECS),
        max_secs.clamp(MIN_SECS, MAX_SECS),
    );
    let local_month = |s: i64| civil_from_days((s + i64::from(utc_offset(s))).div_euclid(86_400));
    let (mut y, mut m) = local_month(lo);
    let end = local_month(hi);
    let mut out = Vec::new();
    while (y, m) <= end {
        let naive = days_from_civil(y, m) * 86_400;
        let guess = naive - i64::from(utc_offset(naive));
        out.push((
            format!("{y:04}-{m:02}"),
            naive - i64::from(utc_offset(guess)),
        ));
        if m == 12 {
            (y, m) = (y + 1, 1);
        } else {
            m += 1;
        }
    }
    out
}

// ---- 수집기 ----

/// 구간 번호는 `bounds.partition_point(|b| b <= v) - 1` (첫 구간 미만은 0). 값 없는 문서는 마지막 칸(`bounds.len()`).
enum Plan {
    Month {
        field: &'static str,
        /// 필드 값 단위당 초 (촬영일: 값이 초 -> 1, 수정일: 값이 ms -> 1000이 나눗수)
        per_sec: i64,
        bounds: Arc<Vec<i64>>,
    },
    Initial {
        field: &'static str,
        /// 구간 1.. 의 시작 (구간 0 은 그 앞 전부)
        bounds: Arc<Vec<u64>>,
    },
}

struct SectionCollector {
    plan: Arc<Plan>,
    slots: usize,
}

enum Column2 {
    I64(Option<Column<i64>>),
    U64(Option<Column<u64>>),
}

struct SectionSegment {
    plan: Arc<Plan>,
    col: Column2,
    counts: Vec<u64>,
}

impl Collector for SectionCollector {
    type Fruit = Vec<u64>;
    type Child = SectionSegment;

    fn for_segment(&self, _ord: u32, segment: &SegmentReader) -> tantivy::Result<SectionSegment> {
        let ff = segment.fast_fields();
        let col = match &*self.plan {
            Plan::Month { field, .. } => Column2::I64(ff.column_opt::<i64>(field)?),
            Plan::Initial { field, .. } => Column2::U64(ff.column_opt::<u64>(field)?),
        };
        Ok(SectionSegment {
            plan: Arc::clone(&self.plan),
            col,
            counts: vec![0; self.slots],
        })
    }

    fn requires_scoring(&self) -> bool {
        false
    }

    fn merge_fruits(&self, fruits: Vec<Vec<u64>>) -> tantivy::Result<Vec<u64>> {
        let mut sum = vec![0u64; self.slots];
        for f in fruits {
            for (s, c) in sum.iter_mut().zip(f) {
                *s += c;
            }
        }
        Ok(sum)
    }
}

impl SegmentCollector for SectionSegment {
    type Fruit = Vec<u64>;

    fn collect(&mut self, doc: DocId, _score: Score) {
        let slot = match (&*self.plan, &self.col) {
            (
                Plan::Month {
                    per_sec, bounds, ..
                },
                Column2::I64(col),
            ) => {
                match col.as_ref().and_then(|c| c.first(doc)) {
                    Some(v) => {
                        // 값 단위 -> 초 (내림) 로 비교한다. 경계는 초 단위다
                        let secs = v.div_euclid(*per_sec);
                        bounds.partition_point(|b| *b <= secs).saturating_sub(1)
                    }
                    None => bounds.len(),
                }
            }
            (Plan::Initial { bounds, .. }, Column2::U64(col)) => {
                // 이름 값이 없는 문서는 빈 이름(0)과 같다
                let hi = col.as_ref().and_then(|c| c.first(doc)).unwrap_or(0);
                bounds.partition_point(|b| *b <= hi)
            }
            _ => return,
        };
        if let Some(c) = self.counts.get_mut(slot) {
            *c += 1;
        }
    }

    fn harvest(self) -> Vec<u64> {
        self.counts
    }
}

impl SearchHandle {
    /// 현재 질의·필터에 대한 구간별 개수. `req.limit`·`cursor`·`at` 는 무시한다.
    /// `utc_offset(utc_초)` = 그 시각의 로컬 UTC 오프셋(초) — 월 경계 계산용 (제품: [`local_utc_offset_secs`]).
    pub fn section_index(
        &self,
        req: &CoordRequest,
        utc_offset: &dyn Fn(i64) -> i32,
    ) -> Result<SectionIndex> {
        let slot = self.read();
        let live = slot.live.as_ref().ok_or(SearchError::NotReady)?;
        let search_req = SearchRequest {
            text: req.text.clone(),
            strategy: req.strategy,
            filters: req.filters.clone(),
            sort: Sort::Relevance,
            limit: 1,
            offset: 0,
        };
        let query = build_query(&live.fields, &live.options, &search_req)?;
        let searcher = live.reader.searcher();
        let generation = slot.generation;
        let desc = req.sort.descending;

        let (kind, keys, plan): (SectionKind, Vec<String>, Option<Plan>) = match req.sort.field {
            SortField::CapturedAt | SortField::CapturedOrModified | SortField::ModifiedAt => {
                let (field, per_sec) = match req.sort.field {
                    SortField::CapturedAt => ("captured_at", 1),
                    SortField::CapturedOrModified => ("captured_or_modified", 1),
                    _ => ("modified_at", 1000),
                };
                let (mut lo, mut hi) = (i64::MAX, i64::MIN);
                for seg in searcher.segment_readers() {
                    if let Some(c) = seg.fast_fields().column_opt::<i64>(field)? {
                        lo = lo.min(c.min_value());
                        hi = hi.max(c.max_value());
                    }
                }
                let months = if lo <= hi {
                    month_bounds(lo.div_euclid(per_sec), hi.div_euclid(per_sec), utc_offset)
                } else {
                    Vec::new()
                };
                let keys = months.iter().map(|(k, _)| k.clone()).collect();
                let bounds = Arc::new(months.iter().map(|(_, s)| *s).collect());
                (
                    SectionKind::Month,
                    keys,
                    Some(Plan::Month {
                        field,
                        per_sec,
                        bounds,
                    }),
                )
            }
            SortField::Name | SortField::Title => {
                let field = if req.sort.field == SortField::Name {
                    "name_hi"
                } else {
                    "title_hi"
                };
                let all = name_initial_bounds();
                let keys = all.iter().map(|(k, _)| k.clone()).collect();
                let bounds = Arc::new(all.iter().skip(1).map(|(_, s)| *s).collect());
                (
                    SectionKind::Initial,
                    keys,
                    Some(Plan::Initial { field, bounds }),
                )
            }
            _ => (SectionKind::None, Vec::new(), None),
        };

        let Some(plan) = plan else {
            let total = searcher.search(query.as_ref(), &tantivy::collector::Count)? as u64;
            return Ok(SectionIndex {
                total,
                generation,
                kind,
                sections: if total == 0 {
                    Vec::new()
                } else {
                    vec![Section {
                        key: "all".into(),
                        count: total,
                    }]
                },
            });
        };
        let slots = match &plan {
            Plan::Month { bounds, .. } => bounds.len() + 1,
            Plan::Initial { bounds, .. } => bounds.len() + 1,
        };
        let counts = searcher.search(
            query.as_ref(),
            &SectionCollector {
                plan: Arc::new(plan),
                slots,
            },
        )?;
        let total = counts.iter().sum();

        // 월: 값 있는 구간을 방향대로, 값 없음은 항상 마지막. 이름: 전체를 방향대로.
        let mut sections: Vec<Section> = Vec::new();
        let mut push = |key: &str, count: u64| {
            if count > 0 {
                sections.push(Section {
                    key: key.to_string(),
                    count,
                });
            }
        };
        match kind {
            SectionKind::Month => {
                let months = keys.len();
                let order: Vec<usize> = if desc {
                    (0..months).rev().collect()
                } else {
                    (0..months).collect()
                };
                for i in order {
                    push(&keys[i], counts[i]);
                }
                push("none", counts[months]);
            }
            _ => {
                let order: Vec<usize> = if desc {
                    (0..keys.len()).rev().collect()
                } else {
                    (0..keys.len()).collect()
                };
                for i in order {
                    push(&keys[i], counts[i]);
                }
            }
        }
        Ok(SectionIndex {
            total,
            generation,
            kind,
            sections,
        })
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy (월 경계·역변환) / Boundary (연말·윤년·음수 시각) / Time (UTC 오프셋 주입)
    use super::*;

    #[test]
    fn t_sect_01_civil_roundtrip_covers_leap_years_and_pre_epoch() {
        for (y, m) in [
            (1970, 1),
            (2000, 2),
            (2024, 2),
            (2024, 12),
            (1969, 12),
            (1900, 3),
            (2100, 2),
        ] {
            let d = days_from_civil(y, m);
            assert_eq!(civil_from_days(d), (y, m), "first day of {y}-{m}");
            assert_eq!(
                civil_from_days(d - 1).1,
                if m == 1 { 12 } else { m - 1 },
                "day before {y}-{m}"
            );
        }
        assert_eq!(days_from_civil(1970, 1), 0);
        assert_eq!(days_from_civil(2024, 3) - days_from_civil(2024, 2), 29);
    }

    #[test]
    fn t_sect_02_month_bounds_follow_the_injected_offset() {
        // Arrange: 2024-01-31T20:00Z 는 UTC 에선 1월, KST(+9) 에선 2월 1일 05:00
        let t = days_from_civil(2024, 1) * 86_400 + 30 * 86_400 + 20 * 3600;
        let utc = month_bounds(t, t, &|_| 0);
        let kst = month_bounds(t, t, &|_| 9 * 3600);
        // Assert
        assert_eq!(
            utc.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(),
            ["2024-01"]
        );
        assert_eq!(
            kst.iter().map(|b| b.0.as_str()).collect::<Vec<_>>(),
            ["2024-02"]
        );
        assert_eq!(
            kst[0].1,
            days_from_civil(2024, 2) * 86_400 - 9 * 3600,
            "KST month starts 9h before UTC midnight"
        );
    }

    #[test]
    fn t_sect_03_month_bounds_span_year_end_and_stay_ascending() {
        let a = days_from_civil(2023, 11) * 86_400 + 5;
        let b = days_from_civil(2024, 2) * 86_400 + 5;
        let v = month_bounds(a, b, &|_| 0);
        let keys: Vec<_> = v.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(keys, ["2023-11", "2023-12", "2024-01", "2024-02"]);
        assert!(v.windows(2).all(|w| w[0].1 < w[1].1));
    }

    #[test]
    fn t_sect_05_local_offset_is_plausible_for_any_instant_and_never_panics() {
        for t in [
            0,
            1_700_000_000,
            4_000_000_000,
            -2_000_000_000,
            i64::MAX,
            i64::MIN,
        ] {
            let o = local_utc_offset_secs(t);
            assert!(o.abs() <= 15 * 3600, "offset {o} at {t}");
        }
    }

    #[test]
    fn t_sect_04_name_bounds_are_strictly_ascending_and_utf8_ordered() {
        let b = name_initial_bounds();
        assert!(
            b.windows(2).all(|w| w[0].1 < w[1].1),
            "bounds must be strictly ascending"
        );
        assert_eq!(b.len(), 1 + 26 + 1 + 14 + 1);
        assert_eq!(b[0].0, "sym");
        assert_eq!(b.last().map(|x| x.0.as_str()), Some("etc_end"));
    }
}
