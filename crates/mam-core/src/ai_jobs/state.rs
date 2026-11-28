//! `ai_task_state` 읽기·쓰기. queued 행 = 작업이 처리할 단계 목록이다 (작업 행에는 단계 정보가 없다).

use rusqlite::{Connection, params};

use super::origin::Origin;
use super::types::{StepKind, StepState, StepStatus};

/// 이 자산에서 처리할 단계 (queued, 그리고 비정상 종료로 남은 running) - 시도 횟수와 함께.
pub fn pending_steps(conn: &Connection, asset_id: i64) -> rusqlite::Result<Vec<(StepKind, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT kind, attempts FROM ai_task_state
         WHERE asset_id = ?1 AND state IN ('queued', 'running')",
    )?;
    let rows = stmt.query_map([asset_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (k, n) = row?;
        if let Some(kind) = StepKind::parse(&k) {
            out.push((kind, n));
        }
    }
    Ok(out)
}

/// 단계 상태를 바꾼다. `attempts` 가 `Some` 이면 그 값으로 덮어쓴다.
pub fn set_step(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    kind: StepKind,
    state: StepState,
    error_code: Option<&str>,
    attempts: Option<i64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE ai_task_state
         SET state = ?3, error_code = ?4, attempts = COALESCE(?5, attempts), updated_at = ?6,
             wait_since = NULL
         WHERE asset_id = ?1 AND kind = ?2",
        params![
            asset_id,
            kind.as_str(),
            state.as_str(),
            error_code,
            attempts,
            now
        ],
    )?;
    Ok(())
}

/// 단계를 queued 로 만든다 (행이 없으면 생성). `force` 가 아니면 이미 done/queued/running 인 단계는 건드리지 않는다.
/// 바뀐 행 수를 돌려준다.
pub fn upsert_queued(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    kinds: &[StepKind],
    force: bool,
    origin: Origin,
) -> rusqlite::Result<usize> {
    // 삭제 표시된 자산(폴더 제거 등)에는 단계를 만들지 않는다
    if !crate::purge::asset_is_active(conn, asset_id)? {
        return Ok(0);
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO ai_task_state (asset_id, kind, state, error_code, attempts, updated_at, origin)
         VALUES (?1, ?2, 'queued', NULL, 0, ?3, ?5)
         ON CONFLICT (asset_id, kind) DO UPDATE SET
           state = 'queued', error_code = NULL, attempts = 0, updated_at = excluded.updated_at,
           origin = excluded.origin, wait_since = NULL
         WHERE ?4 = 1 OR ai_task_state.state IN ('failed', 'skipped')",
    )?;
    let mut changed = 0;
    for k in kinds {
        changed += stmt.execute(params![
            asset_id,
            k.as_str(),
            now,
            i64::from(force),
            origin.as_str()
        ])?;
    }
    if origin == Origin::Manual {
        // 수동 요청이 자동 출처의 대기 행을 덮으면 manual 로 승격한다 (자동 옵션을 꺼도 보존). 반대 방향 강등은 없다
        let mut promote = conn.prepare_cached(
            "UPDATE ai_task_state SET origin = 'manual'
             WHERE asset_id = ?1 AND kind = ?2 AND state IN ('queued', 'running')
               AND origin IS NOT NULL AND origin <> 'manual'",
        )?;
        for k in kinds {
            promote.execute(params![asset_id, k.as_str()])?;
        }
    }
    Ok(changed)
}

/// UI `AnalysisStatus[]` - 6개 종류 모두 (행 없는 종류는 `none`).
pub fn list_status(conn: &Connection, asset_id: i64) -> rusqlite::Result<Vec<StepStatus>> {
    let mut stmt = conn.prepare_cached(
        "SELECT kind, state, error_code, updated_at FROM ai_task_state WHERE asset_id = ?1",
    )?;
    let rows: Vec<(String, String, Option<String>, i64)> = stmt
        .query_map([asset_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(StepKind::ALL
        .into_iter()
        .map(
            |kind| match rows.iter().find(|(k, ..)| k == kind.as_str()) {
                Some((_, state, code, at)) => StepStatus {
                    kind,
                    state: state.clone(),
                    error_code: code.clone(),
                    updated_at: Some(*at),
                },
                None => StepStatus {
                    kind,
                    state: "none".into(),
                    error_code: None,
                    updated_at: None,
                },
            },
        )
        .collect())
}
