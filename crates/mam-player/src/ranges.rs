//! 재생 구간 인자 검증·정규화 — 순수 함수. (프론트 `playRanges.ts` 는 `<video>` 경로가 쓰며 이 crate 와 무관하다)

use serde::Deserialize;

/// 구간 개수 상한 (command 경계에서 검증)
pub const MAX_RANGES: usize = 1000;

/// `player_set_ranges` 인자의 구간 1개 (프론트 `PlayRange { startMs, endMs }`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 개수 <= `MAX_RANGES`, 각 구간 `start >= 0` 그리고 `end > start`
pub fn valid_ranges(ranges: &[PlayRange]) -> bool {
    ranges.len() <= MAX_RANGES
        && ranges
            .iter()
            .all(|r| r.start_ms >= 0 && r.end_ms > r.start_ms)
}

/// 정렬하고 겹치거나 맞닿은 구간을 병합한다. 잘못된 구간(`start < 0` 또는 `end <= start`)은 버린다. 멱등
pub fn normalize(mut ranges: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    ranges.retain(|&(s, e)| s >= 0 && e > s);
    ranges.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match out.last_mut() {
            Some(prev) if s <= prev.1 => prev.1 = prev.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    //! 분류: Validation (정규화) / Boundary (빈 입력)
    use super::*;

    #[test]
    fn t_player_ranges_04b_normalize_sorts_merges_and_drops_invalid() {
        // 정렬 + 겹침 병합 + 맞닿음 병합
        assert_eq!(
            normalize(vec![(5000, 6000), (1000, 2000), (1500, 2500), (2500, 3000)]),
            vec![(1000, 3000), (5000, 6000)]
        );
        // 잘못된 구간은 버린다
        assert_eq!(
            normalize(vec![(-1, 10), (10, 10), (20, 5), (0, 1)]),
            vec![(0, 1)]
        );
        // 빈 입력, 멱등
        assert!(normalize(vec![]).is_empty());
        let once = normalize(vec![(3, 9), (1, 4)]);
        assert_eq!(normalize(once.clone()), once);
    }
}
