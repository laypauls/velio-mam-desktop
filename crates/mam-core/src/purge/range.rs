//! 폴더(또는 볼륨 전체) 범위 한 배치: 범위 안의 위치 행을 지우고, 그 자산의 연결 데이터를 지운다.
//!
//! **대상 한정**: 범위 밖에 활성 위치가 있는 자산(관리 폴더 "경로 변경" 으로 옮겨진 자산의 옛 위치 등)은 지우지 않고
//! 이 범위의 위치 행만 지운다. 자산당 활성 위치는 최대 1개(`uq_file_locations_active_asset`)다.

use std::collections::HashSet;

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use super::linked::purge_assets;
use crate::keyrange::key_range;

/// 삭제 범위: 볼륨 안의 `root_key` 하위(빈 문자열 = 볼륨 전체). `media_types` 가 있으면 그 유형의 자산 위치만.
#[derive(Debug, Clone, Copy)]
pub struct RangeSpec<'a> {
    pub volume_id: i64,
    pub root_key: &'a str,
    /// `media_assets.media_type` 값 (최대 3개)
    pub media_types: Option<&'a [&'a str]>,
}

/// 한 배치 결과. `locations == 0` 이면 범위가 비었다.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RangeBatch {
    /// 지운 위치 행 수
    pub locations: usize,
    /// 연결 데이터를 지운 자산 (캐시 파일 삭제 대상)
    pub purged: Vec<i64>,
    /// 지운 임베딩의 모델 키 (캐시 무효화용)
    pub embedding_keys: Vec<super::EmbeddingKey>,
}

fn select(
    tx: &Connection,
    spec: &RangeSpec<'_>,
    detached: bool,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, i64)>> {
    let (lo, hi) = key_range(spec.root_key);
    // 활성은 `uq_file_locations_active_path`, detach 된 것은 `idx_file_locations_detached_path` 부분 인덱스를 쓴다
    let state = if detached {
        "l.detached_at IS NOT NULL"
    } else {
        "l.detached_at IS NULL"
    };
    let limit = limit as i64;
    match spec.media_types {
        None => {
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT l.location_id, l.asset_id FROM file_locations l
                 WHERE l.volume_id = ?1 AND {state} AND l.path_key >= ?2 AND l.path_key < ?3 LIMIT ?4"
            ))?;
            let rows = stmt.query_map(params![spec.volume_id, lo, hi, limit], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            rows.collect()
        }
        Some(types) => {
            let pick = |i: usize| types.get(i).copied();
            let mut stmt = tx.prepare_cached(&format!(
                "SELECT l.location_id, l.asset_id FROM file_locations l
                 JOIN media_assets a ON a.asset_id = l.asset_id
                 WHERE l.volume_id = ?1 AND {state} AND l.path_key >= ?2 AND l.path_key < ?3
                   AND a.media_type IN (?5, ?6, ?7) LIMIT ?4"
            ))?;
            let rows = stmt.query_map(
                params![spec.volume_id, lo, hi, limit, pick(0), pick(1), pick(2)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            rows.collect()
        }
    }
}

/// 범위 안에 남은 위치 행 수 (활성 + detach).
pub fn count_in_range(conn: &Connection, spec: &RangeSpec<'_>) -> rusqlite::Result<i64> {
    let (lo, hi) = key_range(spec.root_key);
    let mut total = 0i64;
    for state in ["l.detached_at IS NULL", "l.detached_at IS NOT NULL"] {
        total += match spec.media_types {
            None => conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM file_locations l
                     WHERE l.volume_id = ?1 AND {state} AND l.path_key >= ?2 AND l.path_key < ?3"
                ),
                params![spec.volume_id, lo, hi],
                |r| r.get::<_, i64>(0),
            )?,
            Some(types) => {
                let pick = |i: usize| types.get(i).copied();
                conn.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM file_locations l JOIN media_assets a ON a.asset_id = l.asset_id
                         WHERE l.volume_id = ?1 AND {state} AND l.path_key >= ?2 AND l.path_key < ?3
                           AND a.media_type IN (?4, ?5, ?6)"
                    ),
                    params![spec.volume_id, lo, hi, pick(0), pick(1), pick(2)],
                    |r| r.get::<_, i64>(0),
                )?
            }
        };
    }
    Ok(total)
}

fn has_active_elsewhere(
    tx: &Transaction<'_>,
    spec: &RangeSpec<'_>,
    asset_id: i64,
) -> rusqlite::Result<bool> {
    let (lo, hi) = key_range(spec.root_key);
    let mut stmt = tx.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM file_locations
           WHERE asset_id = ?1 AND detached_at IS NULL
             AND NOT (volume_id = ?2 AND path_key >= ?3 AND path_key < ?4))",
    )?;
    stmt.query_row(params![asset_id, spec.volume_id, lo, hi], |r| r.get(0))
}

/// 범위의 위치 행을 최대 `limit` 개 지우고 그 자산의 연결 데이터를 지운다 (한 트랜잭션). `locations == 0` 이 될 때까지 반복 호출한다.
/// 처음부터 다시 불러도 안전하다 — 이미 처리한 자산은 위치 행이 없어 선택되지 않는다.
pub fn purge_range_batch(
    conn: &mut Connection,
    now: i64,
    spec: &RangeSpec<'_>,
    limit: usize,
) -> rusqlite::Result<RangeBatch> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut rows = select(&tx, spec, false, limit)?;
    if rows.len() < limit {
        let rest = limit - rows.len();
        rows.extend(select(&tx, spec, true, rest)?);
    }
    if rows.is_empty() {
        return Ok(RangeBatch::default());
    }
    let mut seen = HashSet::new();
    let mut purge = Vec::new();
    for (_, asset_id) in &rows {
        if seen.insert(*asset_id) && !has_active_elsewhere(&tx, spec, *asset_id)? {
            purge.push(*asset_id);
        }
    }
    let embedding_keys = purge_assets(&tx, now, &purge)?;
    {
        let mut del = tx.prepare_cached("DELETE FROM file_locations WHERE location_id = ?1")?;
        for (location_id, _) in &rows {
            del.execute([location_id])?;
        }
    }
    tx.commit()?;
    Ok(RangeBatch {
        locations: rows.len(),
        purged: purge,
        embedding_keys,
    })
}
