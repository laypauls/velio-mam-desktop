//! 전역 위치 seek (`at`) — 정렬된 결과의 `at` 번째부터 `limit` 건.
//!
//! offset 없는 keyset 방식과 달리 위치만 알 때 쓴다 (목차 레일 점프, 페이지 번호 이동). 상위 `at + limit + 1` 건을
//! 모아야 하므로 비용이 위치에 비례한다. 그래서 결과의 **뒤쪽 절반**은 순서를 뒤집어 끝에서부터
//! `total - at` 건만 모은다 — 수집량은 최대 `total / 2 + limit`.

use super::Run;
use super::key::RankKey;
use crate::error::Result;

/// `(표시 순서의 키, total, start_offset, more)`. `at >= total` 이면 빈 페이지이고 `start_offset = total`.
pub(super) fn page(
    run: &Run<'_>,
    at: usize,
    limit: usize,
) -> Result<(Vec<RankKey>, u64, u64, bool)> {
    let total = run.count()?;
    let at64 = at as u64;
    if at64 >= total {
        return Ok((Vec::new(), total, total, false));
    }
    let more = at64 + (limit as u64) < total;
    if at64 <= total / 2 {
        let mut keys = run.top(at + limit + 1, None, false)?.keys;
        keys.drain(..at.min(keys.len()));
        keys.truncate(limit);
        Ok((keys, total, at64, more))
    } else {
        // 끝에서부터: 수집 순서는 위치 total-1, total-2, ..., at — 뒤집으면 at.. 의 표시 순서
        let mut keys = run.top((total - at64) as usize, None, true)?.keys;
        keys.reverse();
        keys.truncate(limit);
        Ok((keys, total, at64, more))
    }
}
