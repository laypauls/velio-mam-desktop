//! 파일별 분석 이력(`analysis_runs`)과 외부 AI 사용량(`ai_usage`) - 저장·조회 (P3-3).

use mam_db::Clock;
use rusqlite::{Connection, params};
use serde::Serialize;

use super::{AiStoreError, Result};

/// 이력 메시지 상한 (문자 수). 호출자는 정제된 사용자 메시지만 넘긴다 - 여기서는 길이만 방어한다.
pub const MESSAGE_MAX_CHARS: usize = 200;

/// 이력 1건 (한 단계의 한 번 시도).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub asset_id: i64,
    /// describe | keywords | ocr | summary | stt | embedding
    pub kind: String,
    /// openai | gemini | claude | local
    pub provider: String,
    pub ok: bool,
    pub error_code: Option<String>,
    /// 정제된 사용자 메시지 (공급자 응답 본문·키·경로 금지)
    pub message: String,
    pub attempts: i64,
}

/// UI 계약 `AnalysisRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRun {
    pub run_id: i64,
    pub kind: String,
    pub provider: String,
    /// `done` | `failed`
    pub state: &'static str,
    pub error_code: Option<String>,
    pub attempts: i64,
    pub at: i64,
}

/// 실행 이력을 남기고 `run_id` 를 돌려준다. 자산이 `active` 가 아니면(폴더 제거 뒤 늦게 끝난 작업) 아무것도 쓰지 않고 `0`.
pub fn record_run(conn: &Connection, clock: &dyn Clock, rec: &RunRecord) -> Result<i64> {
    if !crate::purge::asset_is_active(conn, rec.asset_id)? {
        return Ok(0);
    }
    let message: String = rec.message.chars().take(MESSAGE_MAX_CHARS).collect();
    Ok(conn.query_row(
        "INSERT INTO analysis_runs (asset_id, kind, provider, ok, error_code, message, attempts, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) RETURNING run_id",
        params![
            rec.asset_id,
            rec.kind,
            rec.provider,
            i64::from(rec.ok),
            rec.error_code,
            message,
            rec.attempts,
            clock.now_ms()
        ],
        |r| r.get(0),
    )?)
}

/// 자산의 이력, 최신순 (`idx_analysis_runs_asset`).
pub fn list_runs(conn: &Connection, asset_id: i64, limit: usize) -> Result<Vec<AnalysisRun>> {
    let mut stmt = conn.prepare_cached(
        "SELECT run_id, kind, provider, ok, error_code, attempts, at FROM analysis_runs
         WHERE asset_id = ?1 ORDER BY at DESC, run_id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        params![asset_id, i64::try_from(limit).unwrap_or(i64::MAX)],
        |r| {
            Ok(AnalysisRun {
                run_id: r.get(0)?,
                kind: r.get(1)?,
                provider: r.get(2)?,
                state: if r.get::<_, i64>(3)? == 1 {
                    "done"
                } else {
                    "failed"
                },
                error_code: r.get(4)?,
                attempts: r.get(5)?,
                at: r.get(6)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 외부 AI 요청 1건의 사용량 (mam-ai `UsageRecord` 와 같은 모양 + 전송한 이미지 수).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageEntry {
    pub provider: String,
    pub model: String,
    pub kind: String,
    pub asset_id: Option<i64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_bytes: u64,
    pub images: u32,
    pub ok: bool,
    pub error_code: Option<String>,
    pub at: i64,
    /// `apiKey` | `subscription` (None = 기록 안 함, 저장 시 NULL)
    pub auth_mode: Option<String>,
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

pub fn record_usage(conn: &Connection, u: &UsageEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO ai_usage
           (provider, model, kind, asset_id, input_tokens, output_tokens, request_bytes, images, ok, error_code, at, auth_mode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            u.provider,
            u.model,
            u.kind,
            u.asset_id,
            to_i64(u.input_tokens),
            to_i64(u.output_tokens),
            to_i64(u.request_bytes),
            i64::from(u.images),
            i64::from(u.ok),
            u.error_code,
            u.at,
            u.auth_mode
        ],
    )?;
    Ok(())
}

/// UI 계약 `AiUsageRow` - 일(로컬)·공급자·모델별 집계.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageRow {
    pub day: String,
    pub provider: String,
    pub model: String,
    /// `apiKey` | `subscription`. 기존 행(NULL)은 `apiKey` 로 합쳐 보고한다.
    pub auth_mode: Option<String>,
    pub requests: i64,
    pub failures: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub images: i64,
}

/// `YYYY-MM-DD` -> 1970-01-01 기준 일수 (그레고리력, Howard Hinnant 알고리즘).
fn epoch_day(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let (y, m, d): (i64, i64, i64) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    );
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || s.len() != 10 {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `list_usage` 의 SQL (EXPLAIN QUERY PLAN 시험용 - 파라미터 ?1 시작 ?2 끝 ?3 오프셋 ms).
pub const USAGE_SQL: &str =
    "SELECT strftime('%Y-%m-%d', (at + ?3) / 1000, 'unixepoch') AS day, provider, model,
            COALESCE(auth_mode, 'apiKey') AS mode,
            COUNT(*), SUM(ok = 0), SUM(input_tokens), SUM(output_tokens), SUM(images)
     FROM ai_usage WHERE at >= ?1 AND at < ?2
     GROUP BY day, provider, model, mode ORDER BY day, provider, model, mode";

/// `[from_day, to_day]`(양끝 포함, 로컬 날짜) 의 일별 집계. `utc_offset_min` = 로컬 - UTC (분).
pub fn list_usage(
    conn: &Connection,
    from_day: &str,
    to_day: &str,
    utc_offset_min: i32,
) -> Result<Vec<AiUsageRow>> {
    let (Some(from), Some(to)) = (epoch_day(from_day), epoch_day(to_day)) else {
        return Err(AiStoreError::Invalid("day must be YYYY-MM-DD"));
    };
    if to < from {
        return Err(AiStoreError::Invalid("to_day before from_day"));
    }
    let off_ms = i64::from(utc_offset_min) * 60_000;
    let start = from * 86_400_000 - off_ms;
    let end = (to + 1) * 86_400_000 - off_ms;
    let mut stmt = conn.prepare_cached(USAGE_SQL)?;
    let rows = stmt.query_map(params![start, end, off_ms], |r| {
        Ok(AiUsageRow {
            day: r.get(0)?,
            provider: r.get(1)?,
            model: r.get(2)?,
            auth_mode: r.get(3)?,
            requests: r.get(4)?,
            failures: r.get(5)?,
            input_tokens: r.get(6)?,
            output_tokens: r.get(7)?,
            images: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
