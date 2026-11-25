//! 전사 조회·구간 찾기·색인 텍스트 (읽기 전용).

use rusqlite::{Connection, OptionalExtension};

use super::{INDEX_TEXT_MAX_CHARS, Result, SegmentHit, TranscriptSegment, TranscriptView};

fn to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

const SEGMENTS_SQL: &str = "SELECT s.start_ms, s.end_ms, s.text
     FROM transcripts t
     JOIN transcript_segments s ON s.transcript_id = t.transcript_id
     WHERE t.asset_id = ?1
     ORDER BY s.seq";

fn segment_rows(conn: &Connection, asset_id: i64) -> Result<Vec<TranscriptSegment>> {
    let mut stmt = conn.prepare_cached(SEGMENTS_SQL)?;
    let rows = stmt.query_map([asset_id], |r| {
        Ok(TranscriptSegment {
            start_ms: to_u64(r.get(0)?),
            end_ms: to_u64(r.get(1)?),
            text: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// 자산의 전사 (없으면 `None`).
pub fn load_transcript(conn: &Connection, asset_id: i64) -> Result<Option<TranscriptView>> {
    let head = conn
        .query_row(
            "SELECT language, text, engine, model, created_at FROM transcripts WHERE asset_id = ?1",
            [asset_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((language, text, engine, model, created_at)) = head else {
        return Ok(None);
    };
    Ok(Some(TranscriptView {
        language,
        text,
        segments: segment_rows(conn, asset_id)?,
        engine,
        model,
        created_at,
    }))
}

/// 질의어 중 하나라도 포함한 구간 (대소문자 무시 부분 일치, 순서대로, 최대 `limit`개).
/// 형태소 단위 일치가 아니라 검색 적중 위치 설명용이다.
pub fn find_segments(
    conn: &Connection,
    asset_id: i64,
    terms: &[&str],
    limit: usize,
) -> Result<Vec<SegmentHit>> {
    let needles: Vec<String> = terms
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if needles.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    Ok(segment_rows(conn, asset_id)?
        .into_iter()
        .filter(|s| {
            let hay = s.text.to_lowercase();
            needles.iter().any(|n| hay.contains(n.as_str()))
        })
        .take(limit)
        .map(|s| SegmentHit {
            start_ms: s.start_ms,
            end_ms: s.end_ms,
            text: s.text,
        })
        .collect())
}

/// `AssetDoc.transcript` 에 넣을 연결 텍스트 (공백으로 이음, 상한 [`INDEX_TEXT_MAX_CHARS`] 문자 - 문자 경계에서 절단).
pub fn transcript_index_text(segments: &[TranscriptSegment]) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    for s in segments {
        let t = s.text.trim();
        if t.is_empty() {
            continue;
        }
        let sep = usize::from(!out.is_empty());
        let remaining = INDEX_TEXT_MAX_CHARS.saturating_sub(chars + sep);
        if remaining == 0 {
            break;
        }
        if sep == 1 {
            out.push(' ');
        }
        let n = t.chars().count();
        if n > remaining {
            out.extend(t.chars().take(remaining));
            tracing::warn!(
                max = INDEX_TEXT_MAX_CHARS,
                "transcript index text truncated"
            );
            break;
        }
        out.push_str(t);
        chars += sep + n;
    }
    out
}
