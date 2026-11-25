//! 디렉터리 단위 배치 reconcile (P2-2) — 호출자가 연 트랜잭션 하나에 여러 파일을 반영한다.
//!
//! 파일마다 SAVEPOINT 를 쓰므로 업무 규칙 위반(`CaseCollision` 등) 한 건은 그 파일만 되돌리고 나머지는 계속한다.
//! 해시가 필요한 파일은 **쓰기 없이** `NeedHash` 로 돌려준다 — 호출자가 쓰기 스레드 밖에서 해시한 뒤
//! `BatchItem::hash` 에 채워 다시 부른다 (해시 I/O 가 쓰기 잠금을 잡지 않는다).

use rusqlite::{Connection, Transaction, TransactionBehavior, params};

use super::store::{self, AssetChange};
use super::{Outcome, Reconciler, ScanEntry, ScanSession, apply};
use crate::error::CoreError;
use crate::hash::{EdgeFile, HashedFile};
use crate::keyrange::key_range;

pub struct BatchItem<'a> {
    pub entry: ScanEntry<'a>,
    pub hash: Option<HashedFile>,
    /// 전체 해시 없는 자산의 내용 변경 판정에 쓰는 새 파일의 앞뒤 4K 체크섬
    pub edge: Option<EdgeFile>,
}

#[derive(Debug)]
pub enum BatchOutcome {
    Done(Outcome),
    /// 전체 내용 해시가 있어야 판정할 수 있다 — DB 는 바뀌지 않았다
    NeedHash,
    /// 새 파일의 앞뒤 4K 체크섬이 있어야 판정할 수 있다 — DB 는 바뀌지 않았다
    NeedEdge,
    /// 이 파일만 실패(DB 변경 없음). 나머지 파일은 반영됐다
    Failed(CoreError),
}

impl Reconciler<'_> {
    /// 배치를 `tx` 안에서 반영한다 (커밋은 호출자). SQLite 자체 오류는 배치 전체 실패로 전파한다.
    pub fn apply_batch(
        &self,
        tx: &mut Transaction<'_>,
        session: &mut ScanSession,
        items: &[BatchItem<'_>],
    ) -> Result<Vec<BatchOutcome>, CoreError> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let sp = tx.savepoint()?;
            let now = self.clock.now_ms();
            // 해시 후 파일이 바뀌었으면 결과를 폐기하고 "봤다" 만 기록한다 (reconcile 과 같은 규칙, D5)
            let meta = &item.entry.meta;
            let stale_hash = item
                .hash
                .is_some_and(|h| h.size != meta.size || h.modified_ms != meta.modified_ms);
            let stale_edge = item
                .edge
                .is_some_and(|e| e.size != meta.size || e.modified_ms != meta.modified_ms);
            if stale_hash || stale_edge {
                let key = mam_db::path_key(item.entry.rel_path)?;
                store::mark_seen_at_path(&sp, session.volume_id, &key, session.scan_at)?;
                sp.commit()?;
                out.push(BatchOutcome::Done(Outcome::HashDeferred));
                continue;
            }
            match apply::apply(
                &sp,
                now,
                session,
                &item.entry,
                item.hash.as_ref(),
                item.edge.as_ref(),
            ) {
                Ok(apply::Step::Done {
                    outcome,
                    consumed_candidate,
                }) => {
                    sp.commit()?;
                    if let Some(location_id) = consumed_candidate {
                        session.consume(location_id);
                    }
                    out.push(BatchOutcome::Done(outcome));
                }
                // sp 는 drop 으로 롤백된다 (NeedHash 는 쓰기가 없다)
                Ok(apply::Step::NeedHash) => out.push(BatchOutcome::NeedHash),
                Ok(apply::Step::NeedEdge) => out.push(BatchOutcome::NeedEdge),
                Err(CoreError::Db(e)) => return Err(CoreError::Db(e)),
                Err(e) => out.push(BatchOutcome::Failed(e)),
            }
        }
        Ok(out)
    }

    /// `finish_scan` 의 폴더 범위판: `root_key` 하위(빈 문자열 = 볼륨 전체)에서 이번 스캔에 못 본 위치만 `missing` 으로 표시한다.
    /// 볼륨이 offline 이면 아무것도 바꾸지 않는다 (0).
    pub fn finish_scan_in(
        &self,
        conn: &mut Connection,
        session: &ScanSession,
        root_key: &str,
    ) -> Result<usize, CoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let online: String = tx.query_row(
            "SELECT availability FROM volumes WHERE volume_id = ?1",
            [session.volume_id],
            |r| r.get(0),
        )?;
        if !session.is_online || online != "online" {
            return Ok(0);
        }
        let (lo, hi) = key_range(root_key);
        let unseen: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT location_id, asset_id FROM file_locations
                 WHERE volume_id = ?1 AND detached_at IS NULL AND presence = 'present'
                   AND path_key >= ?3 AND path_key < ?4 AND last_seen_at < ?2",
            )?;
            let rows = stmt
                .query_map(params![session.volume_id, session.scan_at, lo, hi], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            rows.collect::<Result<_, _>>()?
        };
        let now = self.clock.now_ms();
        for (location_id, asset_id) in &unseen {
            tx.execute(
                "UPDATE file_locations SET presence = 'missing' WHERE location_id = ?1",
                [location_id],
            )?;
            store::update_asset(&tx, now, *asset_id, AssetChange::default())?;
        }
        tx.commit()?;
        Ok(unseen.len())
    }

    /// 감시 이벤트용: `key` 자신(파일)과 그 하위(디렉터리)의 `present` 위치를 `missing` 으로 표시한다.
    /// 볼륨이 offline 이면 아무것도 바꾸지 않는다. 바뀐 개수를 돌려준다.
    pub fn mark_missing_under(
        &self,
        conn: &mut Connection,
        volume_id: i64,
        key: &str,
    ) -> Result<usize, CoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let online: String = tx.query_row(
            "SELECT availability FROM volumes WHERE volume_id = ?1",
            [volume_id],
            |r| r.get(0),
        )?;
        if online != "online" || key.is_empty() {
            return Ok(0);
        }
        let (lo, hi) = key_range(key);
        let mut hits: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT location_id, asset_id FROM file_locations
                 WHERE volume_id = ?1 AND detached_at IS NULL AND presence = 'present'
                   AND path_key >= ?2 AND path_key < ?3",
            )?;
            let rows =
                stmt.query_map(params![volume_id, lo, hi], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        {
            let mut stmt = tx.prepare(
                "SELECT location_id, asset_id FROM file_locations
                 WHERE volume_id = ?1 AND path_key = ?2 AND detached_at IS NULL AND presence = 'present'",
            )?;
            let rows = stmt.query_map(params![volume_id, key], |r| Ok((r.get(0)?, r.get(1)?)))?;
            for row in rows {
                hits.push(row?);
            }
        }
        let now = self.clock.now_ms();
        for (location_id, asset_id) in &hits {
            tx.execute(
                "UPDATE file_locations SET presence = 'missing' WHERE location_id = ?1",
                [location_id],
            )?;
            store::update_asset(&tx, now, *asset_id, AssetChange::default())?;
        }
        tx.commit()?;
        Ok(hits.len())
    }

    /// 폴더 제거용: `root_key` 하위(빈 문자열 = 볼륨 전체)의 활성 위치를 최대 `limit` 개 detach 한다.
    /// 자산 행은 지우지 않고 revision 증가 + outbox 만 남긴다 (태그·메모·AI 결과 보존). 처리한 개수를 돌려준다 —
    /// 0 이 될 때까지 반복 호출한다.
    pub fn detach_under(
        &self,
        conn: &mut Connection,
        volume_id: i64,
        root_key: &str,
        limit: usize,
    ) -> Result<usize, CoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (lo, hi) = key_range(root_key);
        let hits: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT location_id, asset_id FROM file_locations
                 WHERE volume_id = ?1 AND detached_at IS NULL AND path_key >= ?2 AND path_key < ?3
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(params![volume_id, lo, hi, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        let now = self.clock.now_ms();
        for (location_id, asset_id) in &hits {
            store::detach_location(&tx, now, *location_id)?;
            store::update_asset(&tx, now, *asset_id, AssetChange::default())?;
        }
        tx.commit()?;
        Ok(hits.len())
    }

    /// 정리(purge)용: `detach_under` 와 같되 `media_types`(`media_assets.media_type` 값, 최대 3개) 자산의 위치만 대상이다.
    /// 0 이 될 때까지 반복 호출한다. 원본 파일은 건드리지 않는다.
    pub fn detach_under_media(
        &self,
        conn: &mut Connection,
        volume_id: i64,
        root_key: &str,
        media_types: &[&str],
        limit: usize,
    ) -> Result<usize, CoreError> {
        if media_types.is_empty() {
            return Ok(0);
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (lo, hi) = key_range(root_key);
        let pick = |i: usize| media_types.get(i).copied();
        let hits: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT l.location_id, l.asset_id FROM file_locations l
                 JOIN media_assets a ON a.asset_id = l.asset_id
                 WHERE l.volume_id = ?1 AND l.detached_at IS NULL AND l.path_key >= ?2 AND l.path_key < ?3
                   AND a.media_type IN (?5, ?6, ?7)
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![volume_id, lo, hi, limit as i64, pick(0), pick(1), pick(2)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            rows.collect::<Result<_, _>>()?
        };
        let now = self.clock.now_ms();
        for (location_id, asset_id) in &hits {
            store::detach_location(&tx, now, *location_id)?;
            store::update_asset(&tx, now, *asset_id, AssetChange::default())?;
        }
        tx.commit()?;
        Ok(hits.len())
    }
}

/// 읽지 못한 경로(`key` 자신과 하위)의 위치를 "이번 스캔에서 봤다" 로 표시한다 — 읽기 오류가 `missing` 오판이 되지 않게 한다.
/// 자산은 바뀌지 않으므로 revision·outbox 는 없다.
pub fn touch_under(
    conn: &Connection,
    volume_id: i64,
    key: &str,
    seen_at: i64,
) -> Result<(), CoreError> {
    let (lo, hi) = key_range(key);
    conn.execute(
        "UPDATE file_locations SET last_seen_at = ?2
         WHERE volume_id = ?1 AND detached_at IS NULL AND last_seen_at < ?2
           AND path_key >= ?3 AND path_key < ?4",
        params![volume_id, seen_at, lo, hi],
    )?;
    if !key.is_empty() {
        conn.execute(
            "UPDATE file_locations SET last_seen_at = ?3
             WHERE volume_id = ?1 AND path_key = ?2 AND detached_at IS NULL AND last_seen_at < ?3",
            params![volume_id, key, seen_at],
        )?;
    }
    Ok(())
}

/// 폴더의 대상 유형 밖이라 등록하지 않은 파일(`rel_paths` = 볼륨 기준 상대 경로)의 **이미 있는 활성 위치만** "이번 스캔에서 봤다" 로
/// 표시한다 — 유지한 자산이 `missing` 으로 오판되지 않게 한다. 새 자산·위치는 만들지 않는다.
/// `missing` 이던 위치의 파일이 다시 보이면 `present` 로 되돌리고 revision·outbox 를 남긴다.
pub fn touch_excluded_paths(
    conn: &Connection,
    now: i64,
    volume_id: i64,
    rel_paths: &[String],
    seen_at: i64,
) -> Result<(), CoreError> {
    for rel in rel_paths {
        let key = mam_db::path_key(rel)?;
        let Some(loc) = store::active_location_at(conn, volume_id, &key)? else {
            continue;
        };
        if loc.last_seen_at < seen_at {
            conn.execute(
                "UPDATE file_locations SET last_seen_at = ?2 WHERE location_id = ?1",
                params![loc.location_id, seen_at],
            )?;
        }
        if loc.presence == "missing" {
            conn.execute(
                "UPDATE file_locations SET presence = 'present' WHERE location_id = ?1",
                [loc.location_id],
            )?;
            store::update_asset(conn, now, loc.asset_id, AssetChange::default())?;
        }
    }
    Ok(())
}
