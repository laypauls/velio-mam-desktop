//! 장면 검출 작업 등록 — 사용자 요청·백필·자동 등록·pending 재등록 (설계 §4.2).
//!
//! 모두 호출자가 연 트랜잭션 안에서 실행한다(`&Connection` — `Transaction` 은 deref). `jobs` 의 `'scene'` 종류는
//! V16 이후에만 허용된다. 작업 등록 후 `JobQueue::notify()` 는 호출자 몫이다.

use rusqlite::{Connection, OptionalExtension, params};

use super::{SCENE_JOB_KIND, SCENE_PRIORITY_BACKGROUND, SceneError};

/// `asset_scene_cuts.origin` 값 - 자동 등록(메타 완료 직후)
pub const ORIGIN_AUTO: &str = "auto";
/// 사용자 요청·백필 (NULL 구 행도 manual 취급)
pub const ORIGIN_MANUAL: &str = "manual";
use crate::view_options::SceneSensitivity;

/// 백필 한 배치 크기
pub const BACKFILL_BATCH: i64 = 1000;

/// 백필 페이지 질의 — 활성 영상 중 행이 없는(failed 포함 어떤 행도 없는) 자산, `asset_id` 오름차순 키셋
pub const BACKFILL_PAGE_SQL: &str = "SELECT a.asset_id FROM media_assets a
 WHERE a.media_type = 'video' AND a.status = 'active' AND a.asset_id > ?1
   AND NOT EXISTS (SELECT 1 FROM asset_scene_cuts s WHERE s.asset_id = a.asset_id)
 ORDER BY a.asset_id LIMIT ?2";

/// 백필 대상 개수 질의
pub const BACKFILL_COUNT_SQL: &str = "SELECT COUNT(*) FROM media_assets a
 WHERE a.media_type = 'video' AND a.status = 'active'
   AND NOT EXISTS (SELECT 1 FROM asset_scene_cuts s WHERE s.asset_id = a.asset_id)";

/// pending 인데 대기·실행 작업이 없는 활성 자산 질의
pub const REQUEUE_SQL: &str = "SELECT s.asset_id FROM asset_scene_cuts s
 JOIN media_assets a ON a.asset_id = s.asset_id AND a.status = 'active'
 WHERE s.state = 'pending'
   AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.kind = 'scene' AND j.asset_id = s.asset_id
                   AND j.status = 'pending')
   AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.kind = 'scene' AND j.asset_id = s.asset_id
                   AND j.status = 'running')";

/// `scene_detect_request` 옵션 (계약 `SceneDetectOptions`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestOptions {
    pub redetect: bool,
    pub restore_suppressed: bool,
    pub sensitivity: Option<SceneSensitivity>,
}

/// 요청 결과: 대기열에 올라간(이미 대기 중 포함) 수 / 건너뛴 수
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestOutcome {
    pub queued: u32,
    pub skipped: u32,
}

/// 백필 한 배치 결과. `last_id` 는 다음 배치의 `after` (대상이 없으면 `None`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BackfillBatch {
    pub queued: u32,
    pub last_id: Option<i64>,
}

fn insert_job(conn: &Connection, asset_id: i64, priority: i64, now: i64) -> rusqlite::Result<()> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
    )?
    .execute(params![SCENE_JOB_KIND, asset_id, priority, now])?;
    Ok(())
}

/// 사용자 요청 등록. 활성 영상만 대상이며 그 외·이미 자동 추출된 자산(`!redetect`)은 건너뛴다.
/// 대기 중인 작업은 `MIN(priority, priority)` 로 승격한다. 멱등.
pub fn request_tx(
    conn: &Connection,
    asset_ids: &[i64],
    opts: &RequestOptions,
    priority: i64,
    now: i64,
) -> Result<RequestOutcome, SceneError> {
    let mut out = RequestOutcome::default();
    let requested = opts.sensitivity.map(SceneSensitivity::as_str);
    for &id in asset_ids {
        let active_video: bool = conn
            .prepare_cached(
                "SELECT 1 FROM media_assets
                 WHERE asset_id = ?1 AND media_type = 'video' AND status = 'active'",
            )?
            .exists([id])?;
        if !active_video {
            out.skipped += 1;
            continue;
        }
        let existing: Option<(String, Option<String>)> = conn
            .prepare_cached("SELECT state, sensitivity FROM asset_scene_cuts WHERE asset_id = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        if !opts.redetect
            && let Some((state, sens)) = &existing
            && ((state == "done" && sens.is_some()) || state == "unsupported")
        {
            out.skipped += 1;
            continue;
        }
        conn.prepare_cached(
            "INSERT INTO asset_scene_cuts (asset_id, state, requested_sensitivity, updated_at, origin)
             VALUES (?1, 'pending', ?2, ?3, 'manual')
             ON CONFLICT(asset_id) DO UPDATE SET
               state = 'pending', requested_sensitivity = excluded.requested_sensitivity, origin = 'manual',
               error = NULL, updated_at = excluded.updated_at,
               suppressed = CASE WHEN ?4 THEN NULL ELSE suppressed END",
        )?
        .execute(params![id, requested, now, opts.restore_suppressed])?;
        // 수동 재시도 대상이던 실패 작업은 새 요청이 대신한다 (대기 작업과 UNIQUE 충돌 방지)
        conn.prepare_cached(
            "DELETE FROM jobs WHERE kind = 'scene' AND asset_id = ?1 AND status = 'failed'",
        )?
        .execute([id])?;
        let running: bool = conn
            .prepare_cached(
                "SELECT 1 FROM jobs WHERE kind = 'scene' AND asset_id = ?1 AND status = 'running'",
            )?
            .exists([id])?;
        // 실행 중인 작업이 있으면 재검출이 아닌 한 같은 일을 또 올리지 않는다
        if opts.redetect || !running {
            insert_job(conn, id, priority, now)?;
        }
        conn.prepare_cached(
            "UPDATE jobs SET priority = MIN(priority, ?2), updated_at = ?3
             WHERE kind = 'scene' AND asset_id = ?1 AND status = 'pending' AND priority > ?2",
        )?
        .execute(params![id, priority, now])?;
        out.queued += 1;
    }
    Ok(out)
}

/// 백필 한 배치: `after` 뒤의 행 없는 활성 영상 최대 `limit` 건을 pending + 백그라운드 작업으로 등록한다.
pub fn backfill_batch_tx(
    conn: &Connection,
    after: i64,
    limit: i64,
    now: i64,
) -> Result<BackfillBatch, SceneError> {
    let ids: Vec<i64> = conn
        .prepare_cached(BACKFILL_PAGE_SQL)?
        .query_map(params![after, limit], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut ins_row = conn.prepare_cached(
        "INSERT OR IGNORE INTO asset_scene_cuts (asset_id, state, updated_at, origin)
         VALUES (?1, 'pending', ?2, 'manual')",
    )?;
    for &id in &ids {
        ins_row.execute(params![id, now])?;
        insert_job(conn, id, SCENE_PRIORITY_BACKGROUND, now)?;
    }
    Ok(BackfillBatch {
        queued: u32::try_from(ids.len()).unwrap_or(u32::MAX),
        last_id: ids.last().copied(),
    })
}

/// 백필 대상(행 없는 활성 영상) 수
pub fn backfill_count(conn: &Connection) -> Result<i64, SceneError> {
    Ok(conn
        .prepare_cached(BACKFILL_COUNT_SQL)?
        .query_row([], |r| r.get(0))?)
}

/// pending 인데 작업이 없는 활성 자산을 백그라운드 우선순위로 다시 등록한다. 등록 수를 돌려준다.
pub fn requeue_pending_tx(conn: &Connection, now: i64) -> Result<usize, SceneError> {
    let ids: Vec<i64> = conn
        .prepare_cached(REQUEUE_SQL)?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for &id in &ids {
        insert_job(conn, id, SCENE_PRIORITY_BACKGROUND, now)?;
    }
    Ok(ids.len())
}

/// 자동 등록(메타데이터 완료 직후): 행이 아직 없을 때만 pending + 백그라운드 작업. 등록했으면 `true`.
pub fn enqueue_auto_tx(conn: &Connection, asset_id: i64, now: i64) -> Result<bool, SceneError> {
    let inserted = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO asset_scene_cuts (asset_id, state, updated_at, origin)
             VALUES (?1, 'pending', ?2, 'auto')",
        )?
        .execute(params![asset_id, now])?;
    if inserted == 0 {
        return Ok(false);
    }
    insert_job(conn, asset_id, SCENE_PRIORITY_BACKGROUND, now)?;
    Ok(true)
}

/// 자동 추출 옵션이 꺼졌을 때: 자동 등록(`origin = 'auto'`)된 대기(pending) 행과 그 대기 작업을 정리한다
/// (`asset_id` 를 주면 그 자산만). 호출자의 트랜잭션 안에서 부른다. 정리한 행 수(삭제 + done 전환)를 돌려준다. 멱등.
/// - 실행 중(running) 작업은 건드리지 않는다 (실행기가 시작 시 스스로 정리한다).
/// - 컷·숨김이 이미 있는 행(사용자가 pending 중에 편집)은 지우지 않고 `done` 으로 바꿔 편집을 보존한다.
/// - 행을 지우므로 옵션을 다시 켜도 기존 영상은 자동 재등록되지 않는다 (신규 영상만 메타 완료 시 등록, 기존은 백필 명령).
pub fn drop_auto_pending_tx(
    conn: &Connection,
    asset_id: Option<i64>,
    now: i64,
) -> Result<usize, SceneError> {
    // 보간하는 값은 i64 와 고정 상수뿐이다. 전체 범위는 부분 인덱스(state = 'pending')를 쓴다.
    let scope = asset_id.map_or_else(String::new, |id| format!("AND asset_id = {id}"));
    let pending_auto = format!("state = 'pending' AND origin = '{ORIGIN_AUTO}' {scope}");
    conn.execute(
        &format!(
            "DELETE FROM jobs WHERE kind = 'scene' AND status = 'pending'
               AND asset_id IN (SELECT asset_id FROM asset_scene_cuts WHERE {pending_auto})"
        ),
        [],
    )?;
    let kept = conn.execute(
        &format!(
            "UPDATE asset_scene_cuts SET state = 'done', origin = '{ORIGIN_MANUAL}',
                    requested_sensitivity = NULL, updated_at = {now}
             WHERE {pending_auto}
               AND (auto_cuts IS NOT NULL OR user_cuts IS NOT NULL OR suppressed IS NOT NULL)"
        ),
        [],
    )?;
    let deleted = conn.execute(
        &format!("DELETE FROM asset_scene_cuts WHERE {pending_auto}"),
        [],
    )?;
    Ok(kept + deleted)
}
