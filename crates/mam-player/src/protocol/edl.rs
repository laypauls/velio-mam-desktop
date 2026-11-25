//! 재생 구간 → mpv EDL(`edl://`) 과 소스 ↔ EDL 시각 변환 (순수 함수, 계약 §1.4).
//! 호스트·IPC·프론트의 모든 시각은 **소스 타임라인**이고, EDL 타임라인은 워커 안에만 있다.

use super::command::RangeMs;

/// ms → 초 문자열 (`1500` → `1.5`, `2000` → `2`, `10` → `0.01`)
fn secs(ms: i64) -> String {
    let (whole, frac) = (ms / 1000, ms % 1000);
    if frac == 0 {
        return whole.to_string();
    }
    let f = format!("{frac:03}");
    format!("{whole}.{}", f.trim_end_matches('0'))
}

/// `edl://%<경로 바이트 길이>%<경로>,<시작>,<길이>;...` — 길이 접두 표기라 경로의 `,` `;` `:` 를 이스케이프하지 않아도 된다.
/// `start_offset_ms` = 파일 첫 프레임의 pts(`demuxer-start-time`) — 구간은 "파일 시작 = 0" 기준이라 EDL 시작에 더한다.
/// 구간은 정렬·병합 끝난 값이어야 한다 (`EdlMap::new` 가 검증)
pub fn build_edl(path: &str, ranges: &[RangeMs], start_offset_ms: i64) -> String {
    let parts: Vec<String> = ranges
        .iter()
        .map(|&(s, e)| {
            format!(
                "%{}%{path},{},{}",
                path.len(),
                secs(s + start_offset_ms),
                secs(e - s)
            )
        })
        .collect();
    format!("edl://{}", parts.join(";"))
}

/// 소스 시각이 구간 안/사이/뒤 어디에 떨어졌는가
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// 어느 구간 안
    Inside,
    /// 구간 사이(또는 첫 구간 앞) — 다음 구간 시작으로 옮겼다
    NextRange,
    /// 마지막 구간 뒤 — EDL 끝(마지막 구간 끝)으로 옮겼다
    PastEnd,
}

/// 소스 ↔ EDL 시각 매핑 (ms, 소수 허용 — 프레임 시작 시각은 정수 ms 가 아니다)
#[derive(Debug, Clone, PartialEq)]
pub struct EdlMap {
    ranges: Vec<RangeMs>,
    /// 각 구간이 EDL 타임라인에서 시작하는 위치
    offsets: Vec<i64>,
    total: i64,
}

impl EdlMap {
    /// 비어 있거나 정렬·겹침·`end > start >= 0` 을 어기면 None
    pub fn new(ranges: &[RangeMs]) -> Option<Self> {
        if ranges.is_empty() {
            return None;
        }
        let mut offsets = Vec::with_capacity(ranges.len());
        let (mut acc, mut prev_end) = (0i64, -1i64);
        for &(s, e) in ranges {
            if s < 0 || e <= s || s < prev_end {
                return None;
            }
            offsets.push(acc);
            acc = acc.checked_add(e - s)?;
            prev_end = e;
        }
        Some(Self {
            ranges: ranges.to_vec(),
            offsets,
            total: acc,
        })
    }

    /// EDL 전체 길이 (구간 길이의 합)
    pub fn total_ms(&self) -> i64 {
        self.total
    }

    /// 첫 구간 시작 (소스 시각)
    pub fn first_start_ms(&self) -> i64 {
        self.ranges.first().map_or(0, |r| r.0)
    }

    /// 마지막 구간 끝 (소스 시각)
    pub fn last_end_ms(&self) -> i64 {
        self.ranges.last().map_or(0, |r| r.1)
    }

    pub fn ranges(&self) -> &[RangeMs] {
        &self.ranges
    }

    /// 소스 시각 → EDL 시각. 구간 사이는 다음 구간 시작, 마지막 구간 뒤는 EDL 끝
    pub fn source_to_edl(&self, src_ms: f64) -> (f64, Landing) {
        for (i, &(s, e)) in self.ranges.iter().enumerate() {
            if src_ms < s as f64 {
                return (self.offsets[i] as f64, Landing::NextRange);
            }
            if src_ms < e as f64 {
                return (
                    self.offsets[i] as f64 + (src_ms - s as f64),
                    Landing::Inside,
                );
            }
        }
        (self.total as f64, Landing::PastEnd)
    }

    /// EDL 시각 → 소스 시각 (범위 밖은 양 끝으로 보정)
    pub fn edl_to_source(&self, edl_ms: f64) -> f64 {
        if edl_ms <= 0.0 {
            return self.first_start_ms() as f64;
        }
        for (i, &(s, e)) in self.ranges.iter().enumerate() {
            let end_off = (self.offsets[i] + (e - s)) as f64;
            if edl_ms < end_off {
                return s as f64 + (edl_ms - self.offsets[i] as f64);
            }
        }
        self.last_end_ms() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(구간 경계·빈 목록) / Validation(잘못된 구간) / Business rule(간격 이동)

    const R: [RangeMs; 3] = [(1000, 2000), (5000, 5500), (9000, 9250)];

    #[test]
    fn t_player_edl_01_build_uses_length_prefix_and_ms_precision() {
        assert_eq!(
            build_edl(r"C:\a b\x,y;z.mp4", &[(0, 2000), (10_000, 12_500)], 0),
            r"edl://%16%C:\a b\x,y;z.mp4,0,2;%16%C:\a b\x,y;z.mp4,10,2.5"
        );
        assert_eq!(build_edl("a", &[], 0), "edl://");
        assert_eq!(build_edl("a", &[(10, 20)], 0), "edl://%1%a,0.01,0.01");
        // 길이 접두는 문자 수가 아니라 UTF-8 바이트 수
        assert_eq!(
            build_edl("한글.mp4", &[(0, 1000)], 0),
            "edl://%10%한글.mp4,0,1"
        );
        // 시작 오프셋 (첫 프레임 pts 0.105s) 은 구간 시작에만 더해진다
        assert_eq!(
            build_edl("a", &[(0, 1000), (2000, 2500)], 105),
            "edl://%1%a,0.105,1;%1%a,2.105,0.5"
        );
    }

    #[test]
    fn t_player_edl_02_new_rejects_invalid_ranges() {
        assert!(EdlMap::new(&[]).is_none());
        assert!(EdlMap::new(&[(5, 5)]).is_none(), "빈 구간");
        assert!(EdlMap::new(&[(-1, 5)]).is_none());
        assert!(EdlMap::new(&[(10, 20), (5, 8)]).is_none(), "정렬 아님");
        assert!(EdlMap::new(&[(0, 10), (5, 20)]).is_none(), "겹침");
        assert!(EdlMap::new(&[(0, 10), (10, 20)]).is_some(), "맞닿음은 허용");
        assert!(EdlMap::new(&[(0, i64::MAX), (i64::MAX, i64::MAX)]).is_none());
    }

    #[test]
    fn t_player_edl_03_total_and_edges() {
        let m = EdlMap::new(&R).unwrap();
        assert_eq!(m.total_ms(), 1000 + 500 + 250);
        assert_eq!((m.first_start_ms(), m.last_end_ms()), (1000, 9250));
    }

    #[test]
    fn t_player_edl_04_source_to_edl_inside_gap_and_past_end() {
        let m = EdlMap::new(&R).unwrap();
        assert_eq!(m.source_to_edl(1000.0), (0.0, Landing::Inside));
        assert_eq!(m.source_to_edl(1999.5), (999.5, Landing::Inside));
        assert_eq!(m.source_to_edl(5250.0), (1250.0, Landing::Inside));
        assert_eq!(
            m.source_to_edl(0.0),
            (0.0, Landing::NextRange),
            "첫 구간 앞"
        );
        assert_eq!(
            m.source_to_edl(2000.0),
            (1000.0, Landing::NextRange),
            "구간 끝 = 사이"
        );
        assert_eq!(m.source_to_edl(7000.0), (1500.0, Landing::NextRange));
        assert_eq!(m.source_to_edl(9250.0), (1750.0, Landing::PastEnd));
        assert_eq!(m.source_to_edl(1e9), (1750.0, Landing::PastEnd));
    }

    #[test]
    fn t_player_edl_05_edl_to_source_round_trips_inside_ranges() {
        let m = EdlMap::new(&R).unwrap();
        for src in [1000.0, 1500.25, 1999.0, 5000.0, 5499.0, 9000.0, 9249.5] {
            let (edl, landing) = m.source_to_edl(src);
            assert_eq!(landing, Landing::Inside);
            assert_eq!(m.edl_to_source(edl), src, "src={src}");
        }
        assert_eq!(m.edl_to_source(1000.0), 5000.0, "구간 2 시작");
        assert_eq!(m.edl_to_source(-5.0), 1000.0);
        assert_eq!(m.edl_to_source(1750.0), 9250.0, "EDL 끝 = 마지막 구간 끝");
        assert_eq!(m.edl_to_source(1e9), 9250.0);
    }

    #[test]
    fn t_player_edl_06_adjacent_ranges_keep_a_continuous_timeline() {
        let m = EdlMap::new(&[(0, 10), (10, 20)]).unwrap();
        assert_eq!(m.source_to_edl(10.0), (10.0, Landing::Inside));
        assert_eq!(m.edl_to_source(10.0), 10.0);
    }
}
