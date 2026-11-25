//! `watch_pending` SQL — 경로당 1행 UPSERT, 기한 순 조회, 표본·실패 갱신, 시작 복구. 쓰기는 호출자가 쥔 연결/트랜잭션에서만 한다.
//!
//! 처리기가 읽은 뒤 기록기가 같은 경로를 다시 보고할 수 있으므로, 처리기의 갱신·삭제는 읽을 때의 `last_seen_at` 이
//! 그대로일 때만 적용된다(`*_if_unchanged`) — 늦게 온 보고가 조용히 지워지지 않는다.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::scan::IngestError;
use crate::volume::strip_verbatim;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    /// 파일 1건 (또는 폴더 자신 — `apply_path` 가 실제 상태로 판단한다)
    Path,
    /// 하위 폴더 순회 (`apply_path` 의 폴더 순회와 같다)
    Subtree,
}

impl PendingKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Subtree => "subtree",
        }
    }

    fn parse(s: &str) -> Self {
        if s == "subtree" {
            Self::Subtree
        } else {
            Self::Path
        }
    }
}

/// 대기열에 넣을 보고 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub folder_id: i64,
    pub abs_path: String,
    pub path_key: String,
    pub kind: PendingKind,
    /// 보고한 쪽 (감지기 ID 또는 `scan`) — 진단용. 이미 있는 행에는 덮어쓰지 않는다(처음 보고한 쪽을 남긴다)
    pub source: String,
}

impl Incoming {
    /// 절대 경로에서 키를 만든다 (확장 접두 제거 + `path_key`). 빈 경로는 `None`.
    pub fn new(folder_id: i64, abs: &Path, kind: PendingKind, source: &str) -> Option<Self> {
        let abs_path = strip_verbatim(&abs.to_string_lossy());
        let path_key = mam_db::path_key(&abs_path).ok()?;
        Some(Self {
            folder_id,
            abs_path,
            path_key,
            kind,
            source: source.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRow {
    pub folder_id: i64,
    pub path_key: String,
    pub abs_path: String,
    pub kind: PendingKind,
    pub source: String,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub due_at: i64,
    pub sample_size: Option<i64>,
    pub sample_mtime: Option<i64>,
    pub stable_count: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
}

const COLS: &str =
    "folder_id, path_key, abs_path, kind, source, first_seen_at, last_seen_at, due_at,
    sample_size, sample_mtime, stable_count, attempts, last_error";

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PendingRow> {
    Ok(PendingRow {
        folder_id: r.get(0)?,
        path_key: r.get(1)?,
        abs_path: r.get(2)?,
        kind: PendingKind::parse(&r.get::<_, String>(3)?),
        source: r.get(4)?,
        first_seen_at: r.get(5)?,
        last_seen_at: r.get(6)?,
        due_at: r.get(7)?,
        sample_size: r.get(8)?,
        sample_mtime: r.get(9)?,
        stable_count: r.get(10)?,
        attempts: r.get(11)?,
        last_error: r.get(12)?,
    })
}

/// [`upsert_batch`] 결과.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UpsertReport {
    pub written: usize,
    /// 상한에 막혀 새 경로를 받지 못한 폴더 (호출자가 재스캔 1회로 바꾼다)
    pub overflowed: Vec<i64>,
    /// 등록 해제 중이거나 없는 폴더라 버린 보고 수
    pub dropped: usize,
}

/// 보고를 한 트랜잭션에 UPSERT 한다. 이미 있는 경로는 `last_seen_at`·`due_at` 만 늦추고 표본·재시도를 처음으로 되돌린다.
/// 폴더의 행이 `cap` 에 닿으면 그 폴더의 **새** 경로는 받지 않는다(이미 있는 경로의 갱신은 계속 받는다).
pub fn upsert_batch(
    conn: &mut Connection,
    items: &[Incoming],
    now: i64,
    due_at: i64,
    cap: usize,
) -> Result<UpsertReport, IngestError> {
    upsert_batch_with(conn, items, now, due_at, cap, |_| Ok(()))
}

/// [`upsert_batch`] + 같은 트랜잭션 안에서 `also` 를 실행한다 (원격 피드 커서 저장). `also` 가 실패하면 UPSERT 도 함께 롤백된다.
pub fn upsert_batch_with(
    conn: &mut Connection,
    items: &[Incoming],
    now: i64,
    due_at: i64,
    cap: usize,
    also: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>,
) -> Result<UpsertReport, IngestError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut report = UpsertReport::default();
    {
        let mut live: std::collections::HashMap<i64, Option<usize>> =
            std::collections::HashMap::new();
        let mut exists = tx
            .prepare_cached("SELECT 1 FROM watch_pending WHERE folder_id = ?1 AND path_key = ?2")?;
        let mut touch = tx.prepare_cached(
            "UPDATE watch_pending SET abs_path = ?3,
               kind = CASE WHEN kind = 'subtree' OR ?4 = 'subtree' THEN 'subtree' ELSE 'path' END,
               last_seen_at = ?5, due_at = ?6, stable_count = 0, attempts = 0, last_error = NULL
             WHERE folder_id = ?1 AND path_key = ?2",
        )?;
        let mut insert = tx.prepare_cached(
            "INSERT INTO watch_pending (folder_id, path_key, abs_path, kind, source, first_seen_at, last_seen_at, due_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
        )?;
        for it in items {
            // 폴더별 현재 행 수 — None 이면 폴더가 없다/해제 중
            let count = match live.get(&it.folder_id) {
                Some(c) => *c,
                None => {
                    let c = folder_row_count(&tx, it.folder_id)?;
                    live.insert(it.folder_id, c);
                    c
                }
            };
            let Some(count) = count else {
                report.dropped += 1;
                continue;
            };
            let present = exists.exists(params![it.folder_id, it.path_key])?;
            if present {
                touch.execute(params![
                    it.folder_id,
                    it.path_key,
                    it.abs_path,
                    it.kind.as_str(),
                    now,
                    due_at
                ])?;
            } else if count >= cap {
                if !report.overflowed.contains(&it.folder_id) {
                    report.overflowed.push(it.folder_id);
                }
                continue;
            } else {
                insert.execute(params![
                    it.folder_id,
                    it.path_key,
                    it.abs_path,
                    it.kind.as_str(),
                    it.source,
                    now,
                    due_at
                ])?;
                live.insert(it.folder_id, Some(count + 1));
            }
            report.written += 1;
        }
    }
    also(&tx)?;
    tx.commit()?;
    Ok(report)
}

/// 한 트랜잭션에 쓰는 최대 행 수. 묶음이 수만 행이면 한 트랜잭션이 사용자 쓰기(High)를 1초 넘게 막는다(10만 경로 폭주 실측) —
/// 이 크기로 나눠 커밋하고 사이에 쓰기 스레드를 양보한다 (각 청크가 별도 쓰기 작업이라 High 작업이 사이에 끼어든다).
pub const MAX_ROWS_PER_TX: usize = 2_000;

/// `items` 를 `chunk` 행씩 나눠 `f(청크, 마지막인가)` 로 쓰고 보고를 합친다. `f` 는 청크마다 **별도 트랜잭션**을 연다.
/// 커서처럼 "이 묶음이 모두 반영된 뒤에만" 올려야 하는 값은 `마지막인가 == true` 일 때만 같은 트랜잭션에 쓴다 — 중간 청크가 실패하면
/// 마지막 청크는 실행되지 않아 커서가 오르지 않는다(앞 청크의 행은 남지만 `apply_path` 가 멱등이고 원격은 같은 줄을 다시 읽어 UPSERT 하므로 무해하다).
/// 항목이 없어도 `f` 는 빈 청크로 한 번 불린다.
pub fn upsert_chunked(
    items: &[Incoming],
    chunk: usize,
    mut f: impl FnMut(&[Incoming], bool) -> Result<UpsertReport, IngestError>,
) -> Result<UpsertReport, IngestError> {
    let chunk = chunk.max(1);
    let mut total = UpsertReport::default();
    let mut parts: Vec<&[Incoming]> = items.chunks(chunk).collect();
    if parts.is_empty() {
        parts.push(&[]);
    }
    let last = parts.len() - 1;
    for (i, part) in parts.into_iter().enumerate() {
        let r = f(part, i == last)?;
        total.written += r.written;
        total.dropped += r.dropped;
        for fid in r.overflowed {
            if !total.overflowed.contains(&fid) {
                total.overflowed.push(fid);
            }
        }
    }
    Ok(total)
}

/// 폴더의 대기 행 수. 폴더가 없거나 등록 해제 중(`enabled = 0`)이면 `None`.
fn folder_row_count(conn: &Connection, folder_id: i64) -> Result<Option<usize>, IngestError> {
    let live: Option<i64> = conn
        .query_row(
            "SELECT enabled FROM managed_folders WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )
        .optional()?;
    if live != Some(1) {
        return Ok(None);
    }
    Ok(Some(count_for_folder(conn, folder_id)? as usize))
}

pub fn count_for_folder(conn: &Connection, folder_id: i64) -> Result<i64, IngestError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM watch_pending WHERE folder_id = ?1",
        [folder_id],
        |r| r.get(0),
    )?)
}

/// `due_at <= now` 인 행을 기한 순으로 최대 `limit` 개. `skip_folders` 의 행은 건드리지 않는다(꺼내기 준비 중).
pub fn due_rows(
    conn: &Connection,
    now: i64,
    limit: usize,
    skip_folders: &[i64],
) -> Result<Vec<PendingRow>, IngestError> {
    let skip = if skip_folders.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = skip_folders.iter().map(i64::to_string).collect();
        format!("AND folder_id NOT IN ({})", ids.join(","))
    };
    let sql = format!(
        "SELECT {COLS} FROM watch_pending WHERE due_at <= ?1 {skip} ORDER BY due_at, folder_id, path_key LIMIT ?2"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params![now, limit as i64], map_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 처리기가 정한 다음 상태. `seen` = 읽을 때의 `last_seen_at` (그 뒤 다시 보고됐으면 적용하지 않는다).
#[derive(Debug, Clone)]
pub enum Mutation {
    Resample {
        folder_id: i64,
        path_key: String,
        seen: i64,
        size: i64,
        mtime: i64,
        stable: i64,
        due_at: i64,
    },
    Delete {
        folder_id: i64,
        path_key: String,
        seen: i64,
    },
    Fail {
        folder_id: i64,
        path_key: String,
        seen: i64,
        attempts: i64,
        error: String,
        due_at: i64,
    },
}

/// 처리기 한 번의 결과를 한 트랜잭션으로 적용한다. 적용된 행 수를 돌려준다.
pub fn apply_mutations(conn: &mut Connection, muts: &[Mutation]) -> Result<usize, IngestError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let mut n = 0;
    for m in muts {
        n += match m {
            Mutation::Resample {
                folder_id,
                path_key,
                seen,
                size,
                mtime,
                stable,
                due_at,
            } => tx.execute(
                "UPDATE watch_pending SET sample_size = ?4, sample_mtime = ?5, stable_count = ?6, due_at = ?7
                 WHERE folder_id = ?1 AND path_key = ?2 AND last_seen_at = ?3",
                params![folder_id, path_key, seen, size, mtime, stable, due_at],
            )?,
            Mutation::Delete {
                folder_id,
                path_key,
                seen,
            } => tx.execute(
                "DELETE FROM watch_pending WHERE folder_id = ?1 AND path_key = ?2 AND last_seen_at = ?3",
                params![folder_id, path_key, seen],
            )?,
            Mutation::Fail {
                folder_id,
                path_key,
                seen,
                attempts,
                error,
                due_at,
            } => tx.execute(
                "UPDATE watch_pending SET attempts = ?4, last_error = ?5, due_at = ?6
                 WHERE folder_id = ?1 AND path_key = ?2 AND last_seen_at = ?3",
                params![folder_id, path_key, seen, attempts, error, due_at],
            )?,
        };
    }
    tx.commit()?;
    Ok(n)
}

/// 시작 복구: 남은 행의 표본을 버리고 바로 다시 확인하게 한다 (앱이 꺼진 동안 파일이 바뀌었을 수 있다).
pub fn recover_all(conn: &Connection, now: i64) -> Result<usize, IngestError> {
    Ok(conn.execute(
        "UPDATE watch_pending SET stable_count = 0, due_at = ?1",
        [now],
    )?)
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod chunk_tests;
