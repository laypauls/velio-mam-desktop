//! 컬렉션 안 순서 — `collection_assets.position` (V12). 간격 있는 정수 위치(`POS_STEP` 단위)를 쓴다.
//!
//! 이동은 이웃 사이 간격에 균등 삽입하고, 간격이 모자라거나 범위(±2^62)를 넘으면 같은 트랜잭션에서 컬렉션 전체를
//! 다시 번호 매긴다(rebalance). 표준 순서는 `position IS NULL, position, added_at, asset_id` — NULL 은 이전 앱이
//! 추가한 행이며 새 앱의 쓰기(추가·이동) 때 정규화된다 (읽기는 쓰지 않는다).
//!
//! 위치는 검색 문서에 들어가지 않고 조회 때 SQLite 에서 읽으므로 이동은 Outbox·revision 을 건드리지 않는다
//! (`rename_collection` 과 같은 선례). 같은 이동을 두 번 해도 결과가 같다(멱등).

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Deserialize;

use super::collections::require_collection;
use super::{UserData, clean_ids, json_ids, returning_ids};
use crate::scan::IngestError;

pub(super) const POS_STEP: i64 = 1 << 20;
const POS_MAX: i64 = 1 << 62;
const POS_MIN: i64 = -(1 << 62);

/// 이동 위치 (계약 `ReorderTarget`): `{kind:"before",assetId} | {kind:"start"} | {kind:"end"}`.
/// "맨 앞"을 `before(첫 항목)` 으로 표현할 수 없다 — 필터·검색·숨김 때문에 화면의 첫 항목이 진짜 첫 항목이 아닐 수 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ReorderTarget {
    Before { asset_id: i64 },
    Start,
    End,
}

/// 위치 NULL 행이 있으면 컬렉션 전체를 번호 매긴다.
pub(super) fn normalize_if_null(tx: &Connection, cid: i64) -> Result<(), IngestError> {
    let has_null: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM collection_assets WHERE collection_id = ?1 AND position IS NULL)",
        [cid],
        |r| r.get(0),
    )?;
    if has_null {
        rebalance(tx, cid)?;
    }
    Ok(())
}

/// 표준 순서대로 `1..=n` × `POS_STEP` 으로 다시 매긴다.
pub(super) fn rebalance(tx: &Connection, cid: i64) -> Result<(), IngestError> {
    tx.execute(
        "UPDATE collection_assets SET position = r.rn * ?2
         FROM (SELECT asset_id, row_number() OVER (ORDER BY position IS NULL, position, added_at, asset_id) AS rn
                 FROM collection_assets WHERE collection_id = ?1) AS r
         WHERE collection_assets.collection_id = ?1 AND collection_assets.asset_id = r.asset_id",
        params![cid, POS_STEP],
    )?;
    Ok(())
}

fn write_positions(tx: &Connection, cid: i64, pairs: &[(i64, i64)]) -> Result<(), IngestError> {
    // 행마다 기본키 갱신 — `UPDATE .. FROM json_each` 는 계획에 따라 컬렉션 전체(10만)를 훑는다 (측정: k=1 에도 ~50ms)
    let mut stmt = tx.prepare_cached(
        "UPDATE collection_assets SET position = ?3 WHERE collection_id = ?1 AND asset_id = ?2",
    )?;
    for (asset_id, pos) in pairs {
        stmt.execute(params![cid, asset_id, pos])?;
    }
    Ok(())
}

/// 전체 재작성(간격 부족 폴백): 이동 블록(`moved_json`)을 정렬 키 `block_key` 바로 앞에 두고 `rank × POS_STEP` 으로 한 문장에 다시 매긴다.
/// 블록 안은 현재 순서를 유지하고, 같은 키의 기준 항목(`block_key` 와 같은 위치)보다 앞에 놓인다. 위치 NULL 이 없을 때만 호출한다.
fn rebalance_with_block(
    tx: &Connection,
    cid: i64,
    moved_json: &str,
    block_key: i64,
) -> Result<(), IngestError> {
    tx.execute(
        "UPDATE collection_assets SET position = r.rn * ?2
         FROM (SELECT asset_id, row_number() OVER (ORDER BY k, m, position, asset_id) AS rn
                 FROM (SELECT asset_id, position,
                              CASE WHEN asset_id IN (SELECT value FROM json_each(?3)) THEN ?4 ELSE position END AS k,
                              CASE WHEN asset_id IN (SELECT value FROM json_each(?3)) THEN 0 ELSE 1 END AS m
                         FROM collection_assets WHERE collection_id = ?1)) AS r
         WHERE collection_assets.collection_id = ?1 AND collection_assets.asset_id = r.asset_id",
        params![cid, POS_STEP, moved_json, block_key],
    )?;
    Ok(())
}

/// 새 항목을 맨 뒤에 요청 순서대로 붙인다. 이미 있는 항목은 위치를 유지한다. 실제로 추가된 asset_id 를 돌려준다.
pub(super) fn append(
    tx: &Connection,
    cid: i64,
    ids: &[i64],
    now: i64,
) -> Result<Vec<i64>, IngestError> {
    normalize_if_null(tx, cid)?;
    let max_pos = || -> Result<i64, IngestError> {
        Ok(tx.query_row(
            "SELECT COALESCE(max(position), 0) FROM collection_assets WHERE collection_id = ?1",
            [cid],
            |r| r.get(0),
        )?)
    };
    let span = (ids.len() as i64 + 1) * POS_STEP;
    let mut base = max_pos()?;
    if base.saturating_add(span) > POS_MAX {
        rebalance(tx, cid)?;
        base = max_pos()?;
    }
    returning_ids(
        tx,
        "INSERT OR IGNORE INTO collection_assets (collection_id, asset_id, added_at, position)
         SELECT ?2, j.value, ?3, ?4 + (j.key + 1) * ?5 FROM json_each(?1) j WHERE true
         RETURNING asset_id",
        params![json_ids(ids), cid, now, base, POS_STEP],
    )
}

/// 이웃 사이 새 위치 `k` 개. 간격·범위가 모자라면 `None` (호출자가 전체 재배치).
fn gap_positions(lo: Option<i64>, hi: Option<i64>, k: i64) -> Option<Vec<i64>> {
    let out: Vec<i64> = match (lo, hi) {
        (None, None) => (1..=k).map(|i| i * POS_STEP).collect(),
        (Some(lo), None) => (1..=k)
            .map(|i| lo.checked_add(i * POS_STEP))
            .collect::<Option<_>>()?,
        (None, Some(hi)) => (1..=k)
            .map(|i| hi.checked_sub((k + 1 - i) * POS_STEP))
            .collect::<Option<_>>()?,
        (Some(lo), Some(hi)) => {
            let gap = hi.checked_sub(lo)?;
            if gap <= k {
                return None;
            }
            (1..=k).map(|i| lo + i * (gap / (k + 1))).collect()
        }
    };
    out.iter()
        .all(|p| (POS_MIN..=POS_MAX).contains(p))
        .then_some(out)
}

fn neighbour(
    tx: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Option<i64>, IngestError> {
    Ok(tx.query_row(sql, p, |r| r.get::<_, Option<i64>>(0))?)
}

fn reorder(
    tx: &mut Transaction<'_>,
    id: i64,
    ids: &[i64],
    target: ReorderTarget,
) -> Result<(), IngestError> {
    require_collection(tx, id)?;
    normalize_if_null(tx, id)?;
    let moved_json = json_ids(ids);

    // 이동 대상: 현재 위치 순으로 (상대 순서 유지). 소속이 아닌 id 가 있으면 전체 거부
    let moved: Vec<i64> = {
        let mut stmt = tx.prepare_cached(
            "SELECT x.asset_id FROM json_each(?1) j
             CROSS JOIN collection_assets x ON x.collection_id = ?2 AND x.asset_id = j.value
             ORDER BY x.position, x.asset_id",
        )?;
        let rows = stmt.query_map(params![moved_json, id], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    if moved.len() != ids.len() {
        return Err(IngestError::InvalidInput("assetIds"));
    }

    let (lo, hi) = match target {
        ReorderTarget::Before { asset_id } => {
            if moved.contains(&asset_id) {
                return Err(IngestError::InvalidInput("beforeAssetId"));
            }
            let hi: Option<i64> = tx
                .query_row(
                    "SELECT position FROM collection_assets WHERE collection_id = ?1 AND asset_id = ?2",
                    params![id, asset_id],
                    |r| r.get(0),
                )
                .optional()?;
            let hi = hi.ok_or(IngestError::InvalidInput("beforeAssetId"))?;
            let lo = neighbour(
                tx,
                "SELECT max(position) FROM collection_assets WHERE collection_id = ?1 AND position < ?2
                   AND asset_id NOT IN (SELECT value FROM json_each(?3))",
                params![id, hi, moved_json],
            )?;
            (lo, Some(hi))
        }
        ReorderTarget::End => {
            let lo = neighbour(
                tx,
                "SELECT max(position) FROM collection_assets WHERE collection_id = ?1
                   AND asset_id NOT IN (SELECT value FROM json_each(?2))",
                params![id, moved_json],
            )?;
            (lo, None)
        }
        ReorderTarget::Start => {
            let hi = neighbour(
                tx,
                "SELECT min(position) FROM collection_assets WHERE collection_id = ?1
                   AND asset_id NOT IN (SELECT value FROM json_each(?2))",
                params![id, moved_json],
            )?;
            (None, hi)
        }
    };

    if let Some(pos) = gap_positions(lo, hi, moved.len() as i64) {
        let pairs: Vec<(i64, i64)> = moved.iter().copied().zip(pos).collect();
        return write_positions(tx, id, &pairs);
    }
    // 간격 부족 — 이동 블록을 목표 자리(기준 항목 바로 앞·맨 앞·맨 뒤)에 두고 컬렉션 전체를 한 문장으로 다시 번호 매긴다
    let block_key = match target {
        ReorderTarget::Before { .. } => hi.unwrap_or(i64::MAX),
        ReorderTarget::Start => i64::MIN,
        ReorderTarget::End => i64::MAX,
    };
    rebalance_with_block(tx, id, &moved_json, block_key)
}

impl UserData {
    /// 선택한 항목들을 현재 상대 순서를 유지한 채 한 덩어리로 `target` 위치로 옮긴다 (한 트랜잭션, 전부 아니면 전무).
    ///
    /// 오류: 빈 목록·소속 아닌 id → `InvalidInput("assetIds")`, 기준 항목이 이동 대상이거나 소속 아님 →
    /// `InvalidInput("beforeAssetId")`, 컬렉션 없음 → `NotFound`.
    pub fn reorder_collection(
        &self,
        id: i64,
        asset_ids: &[i64],
        target: ReorderTarget,
    ) -> Result<(), IngestError> {
        let ids = clean_ids(asset_ids)?;
        if ids.is_empty() {
            return Err(IngestError::InvalidInput("assetIds"));
        }
        self.write(move |tx, _| reorder(tx, id, &ids, target))
    }
}
