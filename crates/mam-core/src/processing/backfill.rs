//! 누락된 처리 작업을 다시 등록한다.
//!
//! - 앱을 처리 기능 없이 쓰던 기존 DB, 오프라인이라 건너뛴 자산(볼륨이 돌아온 뒤), 비정상 종료로 사라진 작업을 메운다
//! - 기준은 `asset_derivatives` 행 없음(= pending)과 `media_assets` 의 해시 상태다. `failed` 는 건드리지 않는다
//!   (수동 재시도는 [`retry_failed`])
//! - `hash` 는 앞뒤 4K 체크섬(`edge_sig`)이 없는 자산, 또는 전체 해시가 없는데 같은 크기·같은 sig 상대가 있는 자산만 등록한다.
//!   sig 가 있고 상대가 없는 `pending` 자산은 "전체 해시 불필요" 라 다시 등록하지 않는다
//! - 멱등: 같은 (kind, asset) 대기 작업은 중복 등록되지 않는다

use mam_db::{DbPool, Priority};
use rusqlite::{Connection, params};

use super::ProcError;
use super::derivatives::{DerivKind, DerivState, set_state};
use crate::jobs::JobKind;

/// 한 트랜잭션에서 처리하는 자산 수
const BATCH: i64 = 1_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillReport {
    pub meta: usize,
    pub thumb: usize,
    pub hash: usize,
}

/// `kind` 파생물 행이 없는 활성 자산 한 묶음 (asset_id 순, `after` 이후).
fn missing_batch(
    conn: &Connection,
    kind: DerivKind,
    after: i64,
) -> Result<Vec<(i64, String)>, ProcError> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.asset_id, a.media_type FROM media_assets a
         WHERE a.asset_id > ?1 AND a.status = 'active'
           AND NOT EXISTS (SELECT 1 FROM asset_derivatives d WHERE d.asset_id = a.asset_id AND d.kind = ?2)
         ORDER BY a.asset_id LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![after, kind.as_str(), BATCH], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn insert_job(
    conn: &Connection,
    now: i64,
    kind: JobKind,
    asset_id: i64,
) -> Result<bool, ProcError> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![kind.as_str(), asset_id, kind.default_priority(), now],
    )? > 0)
}

/// 처리 작업을 다시 등록한다. 등록한 작업 수를 돌려준다.
pub fn backfill_jobs(pool: &DbPool, now_ms: impl Fn() -> i64) -> Result<BackfillReport, ProcError> {
    let mut report = BackfillReport::default();
    for (deriv, job) in [
        (DerivKind::Meta, JobKind::Meta),
        (DerivKind::Thumb, JobKind::Thumb),
    ] {
        let mut after = 0i64;
        loop {
            let now = now_ms();
            let (last, added) = pool.write(Priority::Low, move |c| {
                Ok((|| -> Result<(Option<i64>, usize), ProcError> {
                    let batch = missing_batch(c, deriv, after)?;
                    let tx = c.transaction()?;
                    let mut added = 0;
                    for (asset_id, media_type) in &batch {
                        if deriv == DerivKind::Thumb && media_type == "audio" {
                            set_state(&tx, now, *asset_id, deriv, DerivState::Unsupported, None)?;
                        } else if insert_job(&tx, now, job, *asset_id)? {
                            added += 1;
                        }
                    }
                    tx.commit()?;
                    Ok((batch.last().map(|(id, _)| *id), added))
                })())
            })??;
            match deriv {
                DerivKind::Meta => report.meta += added,
                _ => report.thumb += added,
            }
            match last {
                Some(id) => after = id,
                None => break,
            }
        }
    }
    let now = now_ms();
    report.hash = pool.write(Priority::Low, move |c| {
        Ok((|| -> Result<usize, ProcError> {
            Ok(c.execute(
                "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
                 SELECT 'hash', a.asset_id, ?1, ?2, ?2 FROM media_assets a
                 WHERE a.status = 'active' AND a.hash_state <> 'failed'
                   AND (a.edge_sig IS NULL
                        OR (a.hash_state = 'pending' AND EXISTS (
                              SELECT 1 FROM media_assets b
                              WHERE b.status = 'active' AND b.file_size = a.file_size
                                AND b.edge_sig = a.edge_sig AND b.asset_id <> a.asset_id)))",
                params![JobKind::Hash.default_priority(), now],
            )?)
        })())
    })??;
    Ok(report)
}

/// 실패한 처리를 대기로 되돌리고 작업을 다시 등록한다 (수동 재시도). `kind = None` 은 전부.
/// 프레임 스트립 실패는 부가 기능이라 대상이 아니다 (썸네일을 다시 만들 때 함께 다시 시도된다). 되돌린 건수를 돌려준다.
pub fn retry_failed(
    pool: &DbPool,
    now_ms: impl Fn() -> i64,
    kind: Option<JobKind>,
) -> Result<usize, ProcError> {
    let (meta, thumb, hash) = match kind {
        None => (true, true, true),
        Some(JobKind::Meta) => (true, false, false),
        Some(JobKind::Thumb) => (false, true, false),
        Some(JobKind::Hash) => (false, false, true),
        Some(_) => return Ok(0),
    };
    let now = now_ms();
    let reset = pool.write(Priority::Low, move |c| {
        Ok((|| -> Result<usize, ProcError> {
            let tx = c.transaction()?;
            let mut n = 0;
            if meta {
                n += tx.execute(
                    "DELETE FROM asset_derivatives WHERE kind = 'meta' AND state = 'failed'",
                    [],
                )?;
            }
            if thumb {
                n += tx.execute(
                    "DELETE FROM asset_derivatives WHERE kind = 'thumb' AND state = 'failed'",
                    [],
                )?;
            }
            if hash {
                n += tx.execute(
                    "UPDATE media_assets SET hash_state = 'pending', updated_at = ?1 WHERE hash_state = 'failed'",
                    [now],
                )?;
            }
            tx.commit()?;
            Ok(n)
        })())
    })??;
    backfill_jobs(pool, now_ms)?;
    Ok(reset)
}
