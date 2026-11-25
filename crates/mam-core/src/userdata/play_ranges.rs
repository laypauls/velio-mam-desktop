//! 재생 구간 (`asset_play_ranges`, V13) — 자산 단위의 `[start_ms, end_ms)` 목록.
//!
//! 구간은 검색 색인 대상이 아니므로 자산 `revision`·`search_outbox` 를 건드리지 않는다. 저장은 항상 정규화된
//! (정렬·병합된) 전체 교체다. 원본 미디어와 무관한 MAM 메타데이터라 원본 파일은 읽지도 쓰지도 않는다.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::UserData;
use super::collections::require_collection;
use crate::scan::IngestError;

/// 자산당 구간 수 상한
pub const MAX_PLAY_RANGES: usize = 1000;
/// fps 를 모를 때의 최소 구간 길이(ms)
const DEFAULT_MIN_LEN_MS: i64 = 100;

/// 계약 `PlayRange` — 밀리초 구간 (`start_ms` 포함, `end_ms` 미포함).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRange {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 계약 `PlayRangeSummary` — 컬렉션 항목 중 구간이 있는 자산의 요약 (희소).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRangeSummary {
    pub asset_id: i64,
    pub range_count: i64,
    pub total_ms: i64,
}

/// 구간 조회 — `(asset_id, start_ms)` PK 범위 검색.
pub const GET_SQL: &str =
    "SELECT start_ms, end_ms FROM asset_play_ranges WHERE asset_id = ?1 ORDER BY start_ms";

/// 구간 테이블을 구동하고 `collection_assets` PK 로 점 조회한다 (비용 ∝ 구간 보유 자산 수).
pub const SUMMARY_SQL: &str = "SELECT r.asset_id, count(*), sum(r.end_ms - r.start_ms)
 FROM asset_play_ranges r
 CROSS JOIN collection_assets x ON x.collection_id = ?1 AND x.asset_id = r.asset_id
 GROUP BY r.asset_id";

/// 최소 구간 길이(ms): fps 가 유효하면 1프레임(`max(1, round(1000/fps))`), 아니면 100ms.
pub fn min_range_len_ms(fps: Option<f64>) -> i64 {
    match fps {
        Some(f) if f.is_finite() && f > 0.0 => {
            // 1000/f 는 f 가 아주 크면 0 에 수렴, 아주 작으면 매우 커진다 — 포화 변환 후 하한 1
            ((1000.0 / f).round() as i64).max(1)
        }
        _ => DEFAULT_MIN_LEN_MS,
    }
}

/// 구간 정규화 (순수 함수). 개수·범위 검증 → 길이 clamp → 정렬·병합(맞닿음 포함) → 최소 길이 검사.
/// 빈 결과는 "모든 구간 삭제"다. 검증 실패는 `InvalidInput("ranges")`.
pub fn normalize_ranges(
    ranges: &[PlayRange],
    duration_ms: Option<i64>,
    fps: Option<f64>,
) -> Result<Vec<PlayRange>, IngestError> {
    if ranges.len() > MAX_PLAY_RANGES {
        return Err(IngestError::InvalidInput("ranges"));
    }
    if ranges
        .iter()
        .any(|r| r.start_ms < 0 || r.end_ms <= r.start_ms)
    {
        return Err(IngestError::InvalidInput("ranges"));
    }
    let duration = duration_ms.filter(|d| *d > 0);
    // (길이로 자른 구간, 자르기 전 끝) — 병합 뒤 "자르기 때문에만 짧아졌는가" 를 가리기 위해 원래 끝을 함께 들고 간다
    let mut v: Vec<(PlayRange, i64)> = ranges
        .iter()
        .filter_map(|r| match duration {
            Some(d) if r.start_ms >= d => None,
            Some(d) => Some((
                PlayRange {
                    start_ms: r.start_ms,
                    end_ms: r.end_ms.min(d),
                },
                r.end_ms,
            )),
            None => Some((*r, r.end_ms)),
        })
        .collect();
    v.sort_unstable_by_key(|(r, _)| (r.start_ms, r.end_ms));
    let mut merged: Vec<(PlayRange, i64)> = Vec::with_capacity(v.len());
    for (r, raw_end) in v {
        match merged.last_mut() {
            Some((last, last_raw)) if r.start_ms <= last.end_ms => {
                last.end_ms = last.end_ms.max(r.end_ms);
                *last_raw = (*last_raw).max(raw_end);
            }
            _ => merged.push((r, raw_end)),
        }
    }
    let min_len = min_range_len_ms(fps);
    let mut out = Vec::with_capacity(merged.len());
    for (r, raw_end) in merged {
        if r.end_ms - r.start_ms >= min_len {
            out.push(r);
        } else if raw_end - r.start_ms >= min_len {
            // 길이로 자르기 때문에만 짧아졌다 — 시작이 길이 안에 있는 구간과 같은 취급으로 버린다 (전체 저장을 거부하지 않는다)
        } else {
            return Err(IngestError::InvalidInput("ranges"));
        }
    }
    Ok(out)
}

fn load(c: &Connection, asset_id: i64) -> Result<Vec<PlayRange>, IngestError> {
    let mut stmt = c.prepare_cached(GET_SQL)?;
    let rows = stmt.query_map(params![asset_id], |r| {
        Ok(PlayRange {
            start_ms: r.get(0)?,
            end_ms: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

impl UserData {
    /// 자산의 구간 (start 오름차순, 없으면 빈 목록). 없는 자산은 `NotFound`.
    pub fn play_ranges_get(&self, asset_id: i64) -> Result<Vec<PlayRange>, IngestError> {
        self.read(|c| {
            let exists: Option<i64> = c
                .query_row(
                    "SELECT asset_id FROM media_assets WHERE asset_id = ?1",
                    [asset_id],
                    |r| r.get(0),
                )
                .optional()?;
            exists.ok_or(IngestError::NotFound)?;
            load(c, asset_id)
        })
    }

    /// 구간 전체 교체 (원자적, 멱등). 정규화된 결과를 돌려준다. 활성 자산만, 이미지는 `invalid_input("mediaType")`.
    pub fn play_ranges_set(
        &self,
        asset_id: i64,
        ranges: &[PlayRange],
    ) -> Result<Vec<PlayRange>, IngestError> {
        let ranges = ranges.to_vec();
        self.write(move |tx, _now| {
            let row: Option<(String, Option<i64>, Option<f64>)> = tx
                .query_row(
                    "SELECT a.media_type, m.duration_ms, m.fps FROM media_assets a
                     LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
                     WHERE a.asset_id = ?1 AND a.status = 'active'",
                    [asset_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let (media_type, duration, fps) = row.ok_or(IngestError::NotFound)?;
            if media_type == "image" {
                return Err(IngestError::InvalidInput("mediaType"));
            }
            let normalized = normalize_ranges(&ranges, duration, fps)?;
            tx.execute(
                "DELETE FROM asset_play_ranges WHERE asset_id = ?1",
                [asset_id],
            )?;
            let mut ins = tx.prepare_cached(
                "INSERT INTO asset_play_ranges (asset_id, start_ms, end_ms) VALUES (?1, ?2, ?3)",
            )?;
            for r in &normalized {
                ins.execute(params![asset_id, r.start_ms, r.end_ms])?;
            }
            Ok(normalized)
        })
    }

    /// 컬렉션 항목 중 구간이 있는 자산만의 요약 (희소). 없는 컬렉션은 `NotFound`.
    pub fn collection_play_range_summary(
        &self,
        collection_id: i64,
    ) -> Result<Vec<PlayRangeSummary>, IngestError> {
        self.read(|c| {
            require_collection(c, collection_id)?;
            let mut stmt = c.prepare_cached(SUMMARY_SQL)?;
            let rows = stmt.query_map(params![collection_id], |r| {
                Ok(PlayRangeSummary {
                    asset_id: r.get(0)?,
                    range_count: r.get(1)?,
                    total_ms: r.get(2)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }
}
