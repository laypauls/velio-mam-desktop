//! `scan_errors` — 스캔 중 못 읽은 파일·폴더 기록. 같은 (폴더, 경로, 코드) 는 한 행에 시도 횟수만 올린다.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use serde::Serialize;

use super::IngestError;
use super::folders::{load_all, path_text};
use crate::CoreError;

/// `list_scan_errors` 항목 (프론트 계약 `ScanError`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanErrorView {
    pub error_id: i64,
    pub folder_id: i64,
    /// 오류가 난 파일·폴더의 절대 경로 (표시용)
    pub path: String,
    pub error_code: String,
    pub message: String,
    pub attempts: i64,
    pub last_at: i64,
}

pub(super) fn io_code(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::NotFound => "not_found",
        _ => "io",
    }
}

pub(super) fn core_code(e: &CoreError) -> &'static str {
    match e {
        CoreError::Io { source, .. } => io_code(source),
        CoreError::CaseCollision { .. } => "case_collision",
        CoreError::PathKey(_) | CoreError::OutsideVolume { .. } => "invalid_path",
        CoreError::ChangedDuringHash(_) => "changed_during_hash",
        _ => "internal",
    }
}

/// 오류를 기록한다. `rel` = 폴더 루트 기준 상대 경로('/' 구분, 루트 자체는 '').
pub fn record_error(
    conn: &Connection,
    folder_id: i64,
    rel: &str,
    code: &str,
    message: &str,
    now: i64,
) -> Result<(), IngestError> {
    conn.execute(
        "INSERT INTO scan_errors (folder_id, rel_path, error_code, message, last_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (folder_id, rel_path, error_code) DO UPDATE SET
           message = excluded.message, attempts = attempts + 1,
           last_at = excluded.last_at, resolved_at = NULL",
        params![folder_id, rel, code, message, now],
    )?;
    Ok(())
}

/// 이번 완료된 스캔에서 다시 나타나지 않은(`last_at < before`) 오류를 해결 처리한다.
pub fn resolve_stale(
    conn: &Connection,
    folder_id: i64,
    before: i64,
    now: i64,
) -> Result<usize, IngestError> {
    Ok(conn.execute(
        "UPDATE scan_errors SET resolved_at = ?3
         WHERE folder_id = ?1 AND resolved_at IS NULL AND last_at < ?2",
        params![folder_id, before, now],
    )?)
}

/// 미해결 오류 목록(최신 순, `error_id` 키셋 페이지). 반환: (항목, 전체 미해결 수, 다음 커서).
pub fn list_errors(
    conn: &Connection,
    after: Option<i64>,
    limit: usize,
) -> Result<(Vec<ScanErrorView>, i64, Option<i64>), IngestError> {
    let roots: HashMap<i64, String> = load_all(conn)?
        .into_iter()
        .map(|f| (f.folder_id, path_text(&f.root_path())))
        .collect();
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM scan_errors WHERE resolved_at IS NULL",
        [],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT error_id, folder_id, rel_path, error_code, message, attempts, last_at
         FROM scan_errors WHERE resolved_at IS NULL AND error_id < ?1
         ORDER BY error_id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        params![after.unwrap_or(i64::MAX), (limit + 1) as i64],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (error_id, folder_id, rel, error_code, message, attempts, last_at) = row?;
        let root = roots.get(&folder_id).cloned().unwrap_or_default();
        let path = if rel.is_empty() {
            root
        } else {
            format!(
                "{}\\{}",
                root.trim_end_matches('\\'),
                rel.replace('/', "\\")
            )
        };
        items.push(ScanErrorView {
            error_id,
            folder_id,
            path,
            error_code,
            message,
            attempts,
            last_at,
        });
    }
    let next = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|e| e.error_id)
    } else {
        None
    };
    Ok((items, total, next))
}

/// 재시도 대상: 선택한(또는 전체) 미해결 오류의 (error_id, folder_id).
pub fn retry_targets(
    conn: &Connection,
    ids: Option<&[i64]>,
) -> Result<Vec<(i64, i64)>, IngestError> {
    let mut stmt = conn.prepare(
        "SELECT error_id, folder_id FROM scan_errors WHERE resolved_at IS NULL ORDER BY error_id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (id, folder) = row?;
        if ids.is_none_or(|ids| ids.contains(&id)) {
            out.push((id, folder));
        }
    }
    Ok(out)
}
