//! 백그라운드 작업 일시정지 상태의 영속화 — `settings` 테이블의 전용 키 `job_pause` (JSON 객체).
//!
//! 비밀번호 해시와 같은 방식으로 [`crate::settings`] 의 `DEFS` 밖에 둔다: `settings::load` 에 보이지 않고
//! 프론트 설정 패치로 읽거나 쓸 수 없다. 작업 종류 이름은 문자열로 보관하고, 해석은 호출자(`mam-core`)가 한다 —
//! 모르는 이름(이후 버전이 쓴 값)은 호출자가 무시한다.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::DbError;

/// `settings.key`
const KEY: &str = "job_pause";

/// 저장되는 일시정지 상태: 전역 여부 + 종류별 일시정지 목록(종류 이름).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobPauseState {
    pub all: bool,
    pub kinds: Vec<String>,
}

/// 저장된 상태를 읽는다. 값이 없거나 JSON 이 깨졌으면 "해제" 상태를 돌려준다(깨진 경우 키 이름만 경고).
pub fn load_job_pause(conn: &Connection) -> Result<JobPauseState, DbError> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(raw) = raw else {
        return Ok(JobPauseState::default());
    };
    Ok(serde_json::from_str(&raw).unwrap_or_else(|_| {
        tracing::warn!(key = KEY, "setting has an invalid value; using default");
        JobPauseState::default()
    }))
}

/// 상태를 저장한다 (없으면 만들고 있으면 덮어쓴다).
pub fn save_job_pause(conn: &Connection, state: &JobPauseState) -> Result<(), DbError> {
    let json = serde_json::to_string(state)
        .map_err(|e| DbError::from(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY, json],
    )?;
    Ok(())
}
