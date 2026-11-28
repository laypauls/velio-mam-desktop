//! 장면 검출 실행기의 DB 접근 (읽기 컨텍스트·결과 기록).

use mam_db::scene_cuts::{self, SceneState};
use mam_db::{DbPool, Priority, SceneSensitivity};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::CoreError;
use crate::processing::{AssetSnapshot, ProcError};

/// 실행 직전에 읽는 값.
pub(super) struct Ctx {
    pub duration_ms: Option<i64>,
    pub fps: Option<f64>,
    /// 메타데이터가 아직 없다(대기 중) — 길이·프레임률을 모르므로 잠시 미룬다
    pub meta_pending: bool,
    /// 이번 요청의 민감도 (`requested_sensitivity`), 없으면 설정값
    pub sensitivity: SceneSensitivity,
}

pub(super) fn load_ctx(c: &Connection, asset_id: i64) -> Result<Ctx, ProcError> {
    let meta: Option<(Option<i64>, Option<f64>)> = c
        .query_row(
            "SELECT duration_ms, fps FROM media_metadata WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let meta_state: Option<String> = c
        .query_row(
            "SELECT state FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'meta'",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?;
    let requested = scene_cuts::get(c, asset_id)?
        .and_then(|row| row.requested_sensitivity)
        .and_then(|s| SceneSensitivity::parse_str(&s));
    let sensitivity = match requested {
        Some(s) => s,
        None => scene_cuts::load_scene_sensitivity(c)?,
    };
    let (duration_ms, fps) = meta.unwrap_or((None, None));
    Ok(Ctx {
        duration_ms: duration_ms.filter(|d| *d > 0),
        fps: fps.filter(|f| f.is_finite() && *f > 0.0),
        // 행이 없거나 pending 이면 아직이다. failed·unsupported 는 길이를 모른 채 진행한다
        meta_pending: matches!(meta_state.as_deref(), None | Some("pending")),
        sensitivity,
    })
}

fn scene_err(e: scene_cuts::SceneError) -> ProcError {
    match e {
        scene_cuts::SceneError::Sqlite(e) => ProcError::Sql(e),
        other => ProcError::Core(CoreError::Internal(other.to_string())),
    }
}

/// 검출 결과를 기록한다. 자산이 그 사이 바뀌었으면 폐기하고 `false`
/// (스냅샷 대조는 `processing::meta_job::commit` 과 같은 방식).
pub(super) fn commit_cuts(
    conn: &mut Connection,
    now: i64,
    asset_id: i64,
    snap: &AssetSnapshot,
    auto: &[i64],
    sensitivity: SceneSensitivity,
    fps: Option<f64>,
) -> Result<bool, ProcError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !still_current(&tx, asset_id, snap) {
        return Ok(false);
    }
    scene_cuts::replace_auto_tx(&tx, asset_id, auto, sensitivity.as_str(), fps, now)
        .map_err(scene_err)?;
    tx.commit()?;
    Ok(true)
}

/// 실패·미지원으로 끝낸다 (기존 컷은 유지). 자산이 사라졌거나 바뀌었으면 쓰지 않고 `false`.
pub(super) fn commit_outcome(
    conn: &mut Connection,
    now: i64,
    asset_id: i64,
    outcome: SceneState,
    error: Option<&str>,
) -> Result<bool, ProcError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let active: bool = tx
        .query_row(
            "SELECT status = 'active' FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if !active {
        return Ok(false);
    }
    scene_cuts::set_outcome_tx(&tx, asset_id, outcome, error, now).map_err(scene_err)?;
    tx.commit()?;
    Ok(true)
}

fn still_current(conn: &Connection, asset_id: i64, snap: &AssetSnapshot) -> bool {
    let current: Option<(String, i64, i64)> = conn
        .query_row(
            "SELECT status, file_size, modified_at FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    current == Some(("active".to_string(), snap.size, snap.modified_at))
}

/// 실행 중인 작업 행의 `not_before` 를 미룬다. 이어서 `Yielded` 로 끝나면 대기열이 시도 횟수를 올리지 않고
/// 행을 `pending` 으로 되돌리며 `not_before` 는 그대로 둔다 (AI 실행기 `defer` 와 같은 방식).
pub(super) fn defer_job(conn: &Connection, job_id: i64, not_before: i64) -> Result<(), ProcError> {
    conn.execute(
        "UPDATE jobs SET not_before = ?2 WHERE job_id = ?1",
        rusqlite::params![job_id, not_before],
    )?;
    Ok(())
}

/// 실행 직전 안전망: 자동 추출 옵션이 꺼져 있고 이 자산이 자동 등록 대기 행이면 지운다 (수동 요청은 그대로). 지웠으면 `true`.
pub(super) fn drop_disabled_auto(
    conn: &mut Connection,
    now: i64,
    asset_id: i64,
) -> Result<bool, ProcError> {
    let tx = conn.transaction()?;
    let dropped = !scene_cuts::load_scene_detect_auto(&tx)?
        && scene_cuts::drop_auto_pending_tx(&tx, Some(asset_id), now).map_err(scene_err)? > 0;
    tx.commit()?;
    Ok(dropped)
}

/// 한 번의 쓰기 트랜잭션이 잡는 최대 행 수 — 수십만 건이 pending 이어도 DB 쓰기 스레드를 오래 붙잡지 않는다
const REQUEUE_CHUNK: usize = 2_000;

/// `pending` 인데 대기·실행 중인 작업이 없는 활성 자산의 작업을 백그라운드 우선순위로 다시 등록한다
/// (오프라인이라 끝낸 작업·사라진 작업 보충, 설계 ADR-4). 멱등. 등록 수를 돌려준다.
/// `mam_db::scene_cuts::REQUEUE_SQL` 을 `LIMIT` 으로 나눠 청크마다 별도 트랜잭션으로 등록한다 — 등록한 행은 다음
/// 청크의 대상에서 빠지므로 끝까지 진행한다 (mam-db 의 `requeue_pending_tx` 는 전체를 한 트랜잭션으로 처리한다).
pub fn requeue_pending(pool: &DbPool, now: i64) -> Result<usize, ProcError> {
    requeue_pending_in_chunks(pool, now, REQUEUE_CHUNK)
}

/// [`requeue_pending`] 의 청크 크기를 정한다 (시험용 - 작은 청크로 여러 트랜잭션 경로를 밟는다).
#[doc(hidden)]
pub fn requeue_pending_in_chunks(
    pool: &DbPool,
    now: i64,
    chunk: usize,
) -> Result<usize, ProcError> {
    let chunk = chunk.max(1);
    // 자동 추출 옵션이 꺼져 있으면 자동 등록분은 재등록하지 않고 지운다 (재시작/볼륨 복귀마다 꺼진 옵션의 작업이 되살아나지 않게)
    pool.write(Priority::Low, move |c| {
        Ok((|| -> Result<(), ProcError> {
            let tx = c.transaction()?;
            if !scene_cuts::load_scene_detect_auto(&tx)? {
                scene_cuts::drop_auto_pending_tx(&tx, None, now).map_err(scene_err)?;
            }
            tx.commit()?;
            Ok(())
        })())
    })??;
    let mut total = 0;
    loop {
        let added = pool.write(Priority::Low, move |c| {
            Ok((|| -> Result<usize, ProcError> {
                let tx = c.transaction()?;
                let ids: Vec<i64> = tx
                    .prepare(&format!("{} LIMIT {chunk}", scene_cuts::REQUEUE_SQL))?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<_, _>>()?;
                for id in &ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?4)",
                        rusqlite::params![
                            scene_cuts::SCENE_JOB_KIND,
                            id,
                            scene_cuts::SCENE_PRIORITY_BACKGROUND,
                            now
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(ids.len())
            })())
        })??;
        total += added;
        if added < chunk {
            return Ok(total);
        }
    }
}
