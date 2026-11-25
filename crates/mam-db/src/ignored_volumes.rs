//! "묻지 않는 디스크" 영속화 — `settings` 테이블의 전용 키 `ignored_volumes` (JSON 배열).
//!
//! 새 디스크 연결 때 "스캔 대상으로 등록할까요?" 에 "기억하고 나중에" 로 답한 디스크 목록이다.
//! `volumes` 테이블에 넣지 않는 이유: 30초 점검이 모든 비네트워크 볼륨을 `went_online` 으로 보고해 처리 작업 보충이
//! 매번 돌기 때문이다. [`crate::job_pause`] 와 같이 `settings::DEFS` 밖에 두어 프론트 설정 패치로 읽거나 쓸 수 없다.
//! 읽기-수정-쓰기는 호출자가 DB 쓰기 스레드(단일 작성자)에서 실행해 직렬화한다.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::DbError;

/// `settings.key`
const KEY: &str = "ignored_volumes";

/// 묻지 않는 디스크 1건 (계약 `IgnoredVolume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredVolume {
    pub volume_key: String,
    pub label: Option<String>,
    pub last_mount_point: Option<String>,
    /// 기억한 시각 (UTC ms)
    pub ignored_at: i64,
}

/// 목록을 읽는다. 값이 없거나 JSON 이 깨졌으면 빈 목록(깨진 경우 키 이름만 경고).
pub fn load_ignored_volumes(conn: &Connection) -> Result<Vec<IgnoredVolume>, DbError> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [KEY], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    Ok(serde_json::from_str(&raw).unwrap_or_else(|_| {
        tracing::warn!(key = KEY, "setting has an invalid value; using default");
        Vec::new()
    }))
}

fn save(conn: &Connection, list: &[IgnoredVolume]) -> Result<(), DbError> {
    let json = serde_json::to_string(list)
        .map_err(|e| DbError::from(rusqlite::Error::ToSqlConversionFailure(Box::new(e))))?;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![KEY, json],
    )?;
    Ok(())
}

/// 추가한다. 같은 `volume_key` 가 이미 있으면 항목을 늘리지 않고 라벨·마운트 지점만 갱신한다(`ignored_at` 은 처음 값 유지).
pub fn add_ignored_volume(conn: &Connection, entry: &IgnoredVolume) -> Result<(), DbError> {
    let mut list = load_ignored_volumes(conn)?;
    match list.iter_mut().find(|v| v.volume_key == entry.volume_key) {
        Some(existing) => {
            existing.label.clone_from(&entry.label);
            existing
                .last_mount_point
                .clone_from(&entry.last_mount_point);
        }
        None => list.push(entry.clone()),
    }
    save(conn, &list)
}

/// 제거한다. 있었으면 true.
pub fn remove_ignored_volume(conn: &Connection, volume_key: &str) -> Result<bool, DbError> {
    let mut list = load_ignored_volumes(conn)?;
    let before = list.len();
    list.retain(|v| v.volume_key != volume_key);
    if list.len() == before {
        return Ok(false);
    }
    save(conn, &list)?;
    Ok(true)
}

/// 연결된 묻지 않는 디스크의 라벨·마운트 지점을 갱신한다 (설정 목록 표시용). 목록에 없거나 값이 같으면 쓰지 않고 false.
pub fn update_ignored_volume(
    conn: &Connection,
    volume_key: &str,
    label: Option<&str>,
    last_mount_point: Option<&str>,
) -> Result<bool, DbError> {
    let mut list = load_ignored_volumes(conn)?;
    let Some(v) = list.iter_mut().find(|v| v.volume_key == volume_key) else {
        return Ok(false);
    };
    if v.label.as_deref() == label && v.last_mount_point.as_deref() == last_mount_point {
        return Ok(false);
    }
    v.label = label.map(str::to_owned);
    v.last_mount_point = last_mount_point.map(str::to_owned);
    save(conn, &list)?;
    Ok(true)
}
