//! SQLite -> `AssetDoc` (자산 + 활성 위치 + 메타데이터 + 태그 + 사용자 데이터).
//!
//! 색인 워커·전체 재구축·테스트가 같은 질의를 쓴다. 활성 자산(`status = 'active'`)이면서
//! 활성 위치(`detached_at IS NULL`)가 있는 자산만 문서가 된다. 메모·즐겨찾기·컬렉션·태그 id 는 P2-7 이,
//! AI 설명·OCR·요약·키워드(`ai_analyses`)와 STT 전사(`transcripts`)는 자산당 최대 1행을 LEFT JOIN 한다 (V6).

use std::collections::HashMap;

use rusqlite::{Connection, Row, params};

use crate::doc::{AssetDoc, MediaType, Presence};
use super::user_values::load_user_values;

/// 자산당 1행 (태그는 상관 부질의로 `char(31)` 구분 연결). 인자는 `media_assets a` 를 만드는 FROM 절.
macro_rules! doc_select {
    ($from:literal) => {
        concat!(
            "SELECT a.asset_id, a.revision, a.media_type, a.file_size,
        l.volume_id, l.rel_path, l.presence,
        m.captured_at, m.duration_ms,
        (SELECT group_concat(t.name, char(31)) FROM asset_tags at
           JOIN tags t ON t.tag_id = at.tag_id WHERE at.asset_id = a.asset_id),
        (SELECT group_concat(at2.tag_id, ',') FROM asset_tags at2 WHERE at2.asset_id = a.asset_id),
        u.memo, u.favorite,
        (SELECT group_concat(ca.collection_id, ',') FROM collection_assets ca WHERE ca.asset_id = a.asset_id),
        l.path_key, f.folder_id, f.path_key, a.modified_at,
        ai.description, ai.ocr_text, ai.summary,
        (SELECT group_concat(je.value, char(31)) FROM json_each(ai.keywords) je),
        substr(tr.text, 1, 200000), u.title, u.rating
 FROM ",
            $from,
            "
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
 LEFT JOIN asset_user_data u ON u.asset_id = a.asset_id
 LEFT JOIN ai_analyses ai ON ai.asset_id = a.asset_id
 LEFT JOIN transcripts tr ON tr.asset_id = a.asset_id
 LEFT JOIN managed_folders f ON f.volume_id = l.volume_id
   AND (f.path_key = '' OR (l.path_key >= f.path_key || '/' AND l.path_key < f.path_key || '0'))
 WHERE a.status = 'active'"
        )
    };
}

/// 전체 스트리밍 질의 (자산 id 순) — 재구축용.
pub const ALL_DOCS_SQL: &str = concat!(doc_select!("media_assets a"), " ORDER BY a.asset_id");

/// 배치 질의: `?1` = asset_id JSON 배열. `json_each` 가 구동 테이블이 되도록 `CROSS JOIN` 으로
/// 순서를 고정한다 (`IN (SELECT ...)` 형태는 SQLite 가 media_assets 전체를 스캔하는 계획을 골랐다 —
/// `t_sync_01b`).
pub const DOCS_BY_IDS_SQL: &str =
    doc_select!("json_each(?1) j CROSS JOIN media_assets a ON a.asset_id = j.value");

/// 자산 행의 현재 revision (활성 여부와 무관, 삭제 처리용): `?1` = asset_id JSON 배열.
pub const REVISIONS_BY_IDS_SQL: &str = "SELECT a.asset_id, a.revision
 FROM json_each(?1) j CROSS JOIN media_assets a ON a.asset_id = j.value";

/// 인덱스별 반영 revision: `?1` = index_name, `?2` = asset_id JSON 배열.
pub const APPLIED_BY_IDS_SQL: &str = "SELECT p.asset_id, p.applied_revision
 FROM json_each(?2) j CROSS JOIN index_progress p ON p.index_name = ?1 AND p.asset_id = j.value";

/// 색인 대상 문서 수 질의 (활성 자산 + 활성 위치).
pub const COUNT_DOCS_SQL: &str = "SELECT count(*) FROM media_assets a
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 WHERE a.status = 'active'";

pub(crate) fn json_ids(ids: &[u64]) -> String {
    let mut s = String::with_capacity(ids.len() * 8 + 2);
    s.push('[');
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&id.to_string());
    }
    s.push(']');
    s
}

pub fn doc_from_row(r: &Row) -> rusqlite::Result<AssetDoc> {
    let rel_path: String = r.get(5)?;
    let media_type = match r.get::<_, String>(2)?.as_str() {
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        _ => MediaType::Image,
    };
    let presence = if r.get::<_, String>(6)? == "missing" {
        Presence::Missing
    } else {
        Presence::Present
    };
    let tags: Option<String> = r.get(9)?;
    Ok(AssetDoc {
        asset_id: r.get::<_, i64>(0)? as u64,
        revision: r.get::<_, i64>(1)? as u64,
        file_name: rel_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .to_string(),
        user_tags: tags
            .map(|t| t.split('\u{1f}').map(str::to_string).collect())
            .unwrap_or_default(),
        ai_keywords: r
            .get::<_, Option<String>>(21)?
            .map(|k| k.split('\u{1f}').map(str::to_string).collect())
            .unwrap_or_default(),
        ai_description: ai_text(r)?,
        user_memo: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
        title: r.get::<_, Option<String>>(23)?.unwrap_or_default(),
        // 전사는 SQL 에서 상한(200k 문자)을 적용한다 (mam_core::ai_store::INDEX_TEXT_MAX_CHARS 와 같은 값)
        transcript: r.get::<_, Option<String>>(22)?.unwrap_or_default(),
        media_type,
        volume_id: r.get::<_, i64>(4)? as u64,
        presence,
        // 인덱스의 captured_at 은 유닉스 초 (SQLite 는 ms)
        captured_at: r.get::<_, Option<i64>>(7)?.map(|ms| ms / 1000),
        file_size: r.get::<_, i64>(3)? as u64,
        duration_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        favorite: r.get::<_, Option<i64>>(12)?.unwrap_or(0) == 1,
        rating: r.get::<_, Option<i64>>(24)?.unwrap_or(0).clamp(0, 5) as u8,
        collection_ids: parse_ids(r.get(13)?),
        tag_ids: parse_ids(r.get(10)?),
        modified_at: r.get(17)?,
        folder_id: r.get::<_, Option<i64>>(15)?.map(|v| v as u64),
        dir_key: dir_key_of(
            &r.get::<_, String>(14)?,
            r.get::<_, Option<String>>(16)?.as_deref(),
        ),
        // 사용자 필드 값은 별도 질의로 채운다 (`load_docs`·재구축). 이 행 단독 변환은 비워 둔다
        user_fields: Vec::new(),
    })
}

/// 파일의 `path_key` 에서 관리 폴더 루트(`root_key`) 기준 상위 디렉터리 키를 뽑는다 (루트 직속 = `""`).
fn dir_key_of(path_key: &str, root_key: Option<&str>) -> String {
    let rel = match root_key {
        Some(root) if !root.is_empty() => path_key
            .strip_prefix(root)
            .and_then(|r| r.strip_prefix('/'))
            .unwrap_or(path_key),
        _ => path_key,
    };
    rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_string()
}

/// `group_concat(id, ',')` 결과 -> id 목록 (숫자가 아닌 조각은 버린다)
fn parse_ids(joined: Option<String>) -> Vec<u64> {
    joined
        .map(|s| s.split(',').filter_map(|p| p.parse().ok()).collect())
        .unwrap_or_default()
}

/// 자산 1건. 비활성(superseded/deleted)이거나 활성 위치가 없으면 `None`.
pub fn load_doc(conn: &Connection, asset_id: u64) -> rusqlite::Result<Option<AssetDoc>> {
    Ok(load_docs(conn, &[asset_id])?.remove(&asset_id))
}

/// 여러 자산을 질의 1회로 읽는다 (N+1 없음). 문서가 되지 않는 자산은 결과에 없다.
pub fn load_docs(conn: &Connection, ids: &[u64]) -> rusqlite::Result<HashMap<u64, AssetDoc>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare_cached(DOCS_BY_IDS_SQL)?;
    let rows = stmt.query_map(params![json_ids(ids)], doc_from_row)?;
    let mut out = HashMap::with_capacity(ids.len());
    for doc in rows {
        let doc = doc?;
        out.insert(doc.asset_id, doc);
    }
    // 사용자 정의 필드 값 — 같은 연결(같은 스냅샷)에서 질의 1회
    for (asset_id, values) in load_user_values(conn, ids)? {
        if let Some(doc) = out.get_mut(&asset_id) {
            doc.user_fields = values;
        }
    }
    Ok(out)
}

/// `media_assets.revision` (행이 있는 자산만).
pub fn load_revisions(conn: &Connection, ids: &[u64]) -> rusqlite::Result<HashMap<u64, u64>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare_cached(REVISIONS_BY_IDS_SQL)?;
    let rows = stmt.query_map(params![json_ids(ids)], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
    })?;
    rows.collect()
}

/// 색인 대상 문서 수.
pub fn count_docs(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(COUNT_DOCS_SQL, [], |r| r.get::<_, i64>(0))
        .map(|n| n as u64)
}

/// AI 설명 필드 = 설명 + OCR + 요약 (비어 있지 않은 것만 줄바꿈으로 연결)
fn ai_text(r: &Row) -> rusqlite::Result<String> {
    let mut out = String::new();
    for idx in [18, 19, 20] {
        if let Some(part) = r.get::<_, Option<String>>(idx)? {
            let part = part.trim();
            if !part.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(part);
            }
        }
    }
    Ok(out)
}
