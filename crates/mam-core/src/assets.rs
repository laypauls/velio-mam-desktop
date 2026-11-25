//! 자산 조회 서비스 경계 — 호출자(AI 서비스·유사 검색·플러그인 호스트)가 SQL 을 직접 쓰지 않도록 한 곳에 모은 읽기 전용 질의.
//!
//! 경로는 이 모듈 밖으로 나가지 않는다: [`brief`] 는 위치의 마지막 구성요소(파일 이름)만 돌려준다.

use rusqlite::{Connection, OptionalExtension, params};

/// [`same_content`] 가 돌려주는 사본 수의 상한 — 같은 해시 사본이 이보다 많아도 이만큼만 담는다.
pub const MAX_SAME_CONTENT: i64 = 1_000;

/// 자산 행이 있는가 (상태 무관 — 삭제 표시된 자산도 `true`).
pub fn exists(conn: &Connection, asset_id: i64) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id = ?1)",
        [asset_id],
        |r| r.get(0),
    )
}

/// 자산 요약 — 전송·표시용 최소 정보 (절대·상대 경로 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetBrief {
    pub media_type: String,
    pub status: String,
    /// 활성 위치 `rel_path` 의 마지막 구성요소 (`/`·`\` 분할). 활성 위치가 없으면 `None`
    pub file_name: Option<String>,
    pub captured_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// 자산 요약. 자산이 없으면 `None`. 상태는 걸러내지 않는다 (호출자가 `status` 로 판단).
pub fn brief(conn: &Connection, asset_id: i64) -> rusqlite::Result<Option<AssetBrief>> {
    conn.query_row(
        "SELECT a.media_type, a.status,
                (SELECT l.rel_path FROM file_locations l
                  WHERE l.asset_id = a.asset_id AND l.detached_at IS NULL LIMIT 1),
                m.captured_at, m.duration_ms
         FROM media_assets a LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
         WHERE a.asset_id = ?1",
        [asset_id],
        |r| {
            let rel_path: Option<String> = r.get(2)?;
            Ok(AssetBrief {
                media_type: r.get(0)?,
                status: r.get(1)?,
                file_name: rel_path.map(|p| p.rsplit(['/', '\\']).next().unwrap_or("").to_owned()),
                captured_at: r.get(3)?,
                duration_ms: r.get(4)?,
            })
        },
    )
    .optional()
}

/// 기준 자산 자신 + 같은 전체 내용 해시의 다른 자산 (`idx_media_assets_hash`, 사본은 [`MAX_SAME_CONTENT`] 개까지).
/// 자산이 없거나 활성이 아니면 `None`. 해시가 아직 없으면 자신만 담는다.
pub fn same_content(conn: &Connection, asset_id: i64) -> rusqlite::Result<Option<Vec<i64>>> {
    let row: Option<(String, Option<String>, Option<Vec<u8>>)> = conn
        .query_row(
            "SELECT status, hash_algo, content_hash FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((status, algo, hash)) = row else {
        return Ok(None);
    };
    if status != "active" {
        return Ok(None);
    }
    let mut ids = vec![asset_id];
    if let (Some(algo), Some(hash)) = (algo, hash) {
        let mut stmt = conn.prepare_cached(
            "SELECT asset_id FROM media_assets
             WHERE hash_algo = ?1 AND content_hash = ?2 AND asset_id <> ?3 LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![algo, hash, asset_id, MAX_SAME_CONTENT], |r| {
            r.get::<_, i64>(0)
        })?;
        for id in rows {
            ids.push(id?);
        }
    }
    Ok(Some(ids))
}

/// 파일 이름 최대 글자 수 (`mam_ai::external::SanitizedMeta::new` 와 같다 — 동등성은 시험이 확인)
pub const MAX_FILE_NAME_CHARS: usize = 200;

/// 경로 구분자(`/`·`\`)로 나눈 마지막 구성요소에서 제어 문자를 지우고 [`MAX_FILE_NAME_CHARS`] 글자로 자른다. 비면 `None`.
pub fn sanitize_file_name(name_or_path: &str) -> Option<String> {
    let base = name_or_path.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_FILE_NAME_CHARS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// 플러그인 호스트용 자산 상세 — 경로·볼륨 이름·해시·사용자 작성 필드(태그·제목·메모·평점)는 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetDetail {
    pub media_type: String,
    pub status: String,
    /// `"online"` | `"offline"` — 활성 위치가 없으면 도달할 수 없으므로 `"offline"`
    pub availability: String,
    /// 활성 위치의 `"present"` | `"missing"`. 활성 위치가 없으면 `None`
    pub presence: Option<String>,
    pub file_name: Option<String>,
    pub captured_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// 자산 상세. 자산이 없으면 `None`. 상태는 걸러내지 않는다. 활성 위치는 `location_id` 가 가장 작은 하나(결정적).
pub fn detail(conn: &Connection, asset_id: i64) -> rusqlite::Result<Option<AssetDetail>> {
    conn.prepare_cached(
        "SELECT a.media_type, a.status, m.captured_at, m.duration_ms, m.width, m.height,
                l.rel_path, l.presence, v.availability
         FROM media_assets a
         LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
         LEFT JOIN file_locations l ON l.location_id =
             (SELECT location_id FROM file_locations WHERE asset_id = a.asset_id AND detached_at IS NULL ORDER BY location_id LIMIT 1)
         LEFT JOIN volumes v ON v.volume_id = l.volume_id
         WHERE a.asset_id = ?1",
    )?
    .query_row([asset_id], |r| {
        let rel_path: Option<String> = r.get(6)?;
        let availability: Option<String> = r.get(8)?;
        Ok(AssetDetail {
            media_type: r.get(0)?,
            status: r.get(1)?,
            availability: availability.unwrap_or_else(|| "offline".to_owned()),
            presence: r.get(7)?,
            file_name: rel_path.as_deref().and_then(sanitize_file_name),
            captured_at: r.get(2)?,
            duration_ms: r.get(3)?,
            width: r.get(4)?,
            height: r.get(5)?,
        })
    })
    .optional()
}
