//! 파생물(메타·썸네일·프레임 스트립) 상태 — `asset_derivatives`. 행이 없으면 `pending`.
//! 파일 존재만으로 판단하지 않는다 (D3). 작업 등록 헬퍼는 스캔의 자산 변경과 **같은 트랜잭션**에서 호출한다.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::jobs::JobKind;
use crate::reconcile::{MediaType, Outcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DerivKind {
    Meta,
    Thumb,
    Strip,
}

impl DerivKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Meta => "meta",
            Self::Thumb => "thumb",
            Self::Strip => "strip",
        }
    }
}

/// 프론트 계약 `DerivativeState` 와 같은 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DerivState {
    Pending,
    Done,
    Failed,
    Unsupported,
}

impl DerivState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }
}

/// 오류 문구 최대 길이 (경로·stderr 가 길 수 있다)
const MAX_ERROR_LEN: usize = 500;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_ERROR_LEN {
        s.to_string()
    } else {
        s.chars().take(MAX_ERROR_LEN).collect::<String>() + "..."
    }
}

/// 상태를 기록한다 (UPSERT - 같은 작업을 다시 실행해도 행이 늘지 않는다).
pub(crate) fn set_state(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    kind: DerivKind,
    state: DerivState,
    error: Option<&str>,
) -> Result<(), CoreError> {
    // 삭제 표시된 자산에는 늦게 끝난 작업도 상태를 남기지 않는다 (폴더 제거 뒤 연결 행이 되살아나는 것 방지)
    if !crate::purge::asset_is_active(conn, asset_id)? {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO asset_derivatives (asset_id, kind, state, error, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (asset_id, kind) DO UPDATE SET
           state = excluded.state, error = excluded.error, updated_at = excluded.updated_at",
        params![
            asset_id,
            kind.as_str(),
            state.as_str(),
            error.map(clip),
            now
        ],
    )?;
    Ok(())
}

/// 자산 여러 개의 한 종류 상태 (행이 없으면 `Pending`). 검색 결과 한 페이지처럼 최종 후보에 대해서만 부른다.
pub fn states_for(
    conn: &Connection,
    kind: DerivKind,
    asset_ids: &[i64],
) -> Result<HashMap<i64, DerivState>, CoreError> {
    let mut out: HashMap<i64, DerivState> = asset_ids
        .iter()
        .map(|id| (*id, DerivState::Pending))
        .collect();
    let mut stmt =
        conn.prepare("SELECT state FROM asset_derivatives WHERE asset_id = ?1 AND kind = ?2")?;
    for id in asset_ids {
        let mut rows = stmt.query(params![id, kind.as_str()])?;
        if let Some(row) = rows.next()? {
            let s: String = row.get(0)?;
            if let Some(s) = DerivState::parse(&s) {
                out.insert(*id, s);
            }
        }
    }
    Ok(out)
}

/// 대기 작업을 등록한다 (이미 같은 대기 작업이 있으면 무시). 호출자의 트랜잭션 안에서.
fn enqueue(conn: &Connection, now: i64, kind: JobKind, asset_id: i64) -> Result<(), CoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![kind.as_str(), asset_id, kind.default_priority(), now],
    )?;
    Ok(())
}

/// 새(또는 내용이 바뀐) 자산의 처리 작업을 등록한다: 메타 -> 썸네일 -> 해시 (D1).
/// 음성은 썸네일이 없으므로 `unsupported` 로 바로 기록한다. `hash` 작업은 전체 해시가 `pending` 이거나 앞뒤 체크섬이 아직 없을 때 등록한다.
pub(crate) fn schedule(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    media: MediaType,
) -> Result<(), CoreError> {
    enqueue(conn, now, JobKind::Meta, asset_id)?;
    if media == MediaType::Audio {
        set_state(
            conn,
            now,
            asset_id,
            DerivKind::Thumb,
            DerivState::Unsupported,
            None,
        )?;
    } else {
        enqueue(conn, now, JobKind::Thumb, asset_id)?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
         SELECT 'hash', asset_id, ?2, ?3, ?3 FROM media_assets
         WHERE asset_id = ?1 AND (hash_state = 'pending' OR edge_sig IS NULL)",
        params![asset_id, JobKind::Hash.default_priority(), now],
    )?;
    Ok(())
}

/// 같은 자산의 내용이 바뀌었다 (해시 전 변경): 파생물 상태를 모두 `pending` 으로 되돌리고 다시 등록한다.
pub(crate) fn reschedule(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    media: MediaType,
) -> Result<(), CoreError> {
    conn.execute(
        "DELETE FROM asset_derivatives WHERE asset_id = ?1",
        [asset_id],
    )?;
    schedule(conn, now, asset_id, media)
}

/// 스캔 판정 결과에 따라 작업을 등록한다 (스캔 배치 트랜잭션 안에서 호출).
/// 신규·내용 변경으로 생긴 새 자산은 처음부터, 해시 전에 내용이 바뀐 같은 자산은 파생물을 초기화하고 다시 등록한다.
/// 이동·터치·재등장 등 내용이 같은 판정은 건드리지 않는다.
pub(crate) fn schedule_for_outcome(
    conn: &Connection,
    now: i64,
    outcome: Outcome,
    media: MediaType,
) -> Result<(), CoreError> {
    match outcome {
        Outcome::Created { asset_id }
        | Outcome::ContentChanged {
            new_asset_id: asset_id,
            ..
        } => schedule(conn, now, asset_id, media),
        Outcome::PendingUpdated { asset_id } => reschedule(conn, now, asset_id, media),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary (오류 문구 자르기) / Validation (상태 문자열)
    use super::*;

    #[test]
    fn t_proc_deriv_01_clip_limits_error_length_on_char_boundary() {
        let long = "가".repeat(MAX_ERROR_LEN + 50);
        let clipped = clip(&long);
        assert_eq!(clipped.chars().count(), MAX_ERROR_LEN + 3);
        assert!(clipped.ends_with("..."));
        assert_eq!(clip("short"), "short");
    }

    #[test]
    fn t_proc_deriv_02_state_strings_round_trip() {
        for s in [
            DerivState::Pending,
            DerivState::Done,
            DerivState::Failed,
            DerivState::Unsupported,
        ] {
            assert_eq!(DerivState::parse(s.as_str()), Some(s));
        }
        assert_eq!(DerivState::parse("running"), None);
    }
}
