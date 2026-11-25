//! 페이지 하이드레이션·상세 조회 (SQLite). 검색 한 페이지는 질의 1회로 구성한다 (N+1 없음).
//! `json_each(?1) CROSS JOIN media_assets` 로 구동 테이블을 고정한다 (`IN (SELECT ...)` 는 전체 스캔을 고른 전례가 있다).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::types::{
    AiCardState, AssetDetail, AssetSummary, Availability, CollectionRef, MediaKind, TagRef,
};
use crate::processing::media_info;
use crate::scan::IngestError;
use crate::userdata::json_ids;

/// 검색 결과 구성용: 활성 자산 + 활성 위치. `?1` = asset_id JSON 배열.
pub const PAGE_SQL: &str =
    "SELECT a.asset_id, l.rel_path, a.media_type, v.availability, l.presence,
        m.width, m.height, m.duration_ms, a.file_size, m.captured_at, a.modified_at,
        COALESCE(u.favorite, 0), COALESCE(d.state, 'pending'), a.revision, u.title, COALESCE(u.rating, 0),
        CASE WHEN EXISTS (SELECT 1 FROM ai_analyses WHERE asset_id = a.asset_id)
               OR EXISTS (SELECT 1 FROM transcripts WHERE asset_id = a.asset_id) THEN 1
             WHEN EXISTS (SELECT 1 FROM ai_task_state WHERE asset_id = a.asset_id
               AND kind <> 'embedding' AND state = 'failed') THEN 2
             ELSE 0 END
 FROM json_each(?1) j
 CROSS JOIN media_assets a ON a.asset_id = j.value
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 JOIN volumes v ON v.volume_id = l.volume_id
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
 LEFT JOIN asset_user_data u ON u.asset_id = a.asset_id
 LEFT JOIN asset_derivatives d ON d.asset_id = a.asset_id AND d.kind = 'thumb'
 WHERE a.status = 'active'";

/// 상세의 관련 자산(이전/다음 버전)과 본인용: 상태·위치 유무와 관계없이 가장 최근 위치를 쓴다.
/// 위치가 없으면 파일명은 빈 문자열.
pub const RELATED_SQL: &str =
    "SELECT a.asset_id, COALESCE(l.rel_path, ''), a.media_type, COALESCE(v.availability, 'online'),
        COALESCE(l.presence, 'present'),
        m.width, m.height, m.duration_ms, a.file_size, m.captured_at, a.modified_at,
        COALESCE(u.favorite, 0), COALESCE(d.state, 'pending'), a.revision, u.title, COALESCE(u.rating, 0),
        CASE WHEN EXISTS (SELECT 1 FROM ai_analyses WHERE asset_id = a.asset_id)
               OR EXISTS (SELECT 1 FROM transcripts WHERE asset_id = a.asset_id) THEN 1
             WHEN EXISTS (SELECT 1 FROM ai_task_state WHERE asset_id = a.asset_id
               AND kind <> 'embedding' AND state = 'failed') THEN 2
             ELSE 0 END
 FROM json_each(?1) j
 CROSS JOIN media_assets a ON a.asset_id = j.value
 LEFT JOIN file_locations l ON l.location_id = (
        SELECT location_id FROM file_locations WHERE asset_id = a.asset_id
        ORDER BY detached_at IS NULL DESC, location_id DESC LIMIT 1)
 LEFT JOIN volumes v ON v.volume_id = l.volume_id
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
 LEFT JOIN asset_user_data u ON u.asset_id = a.asset_id
 LEFT JOIN asset_derivatives d ON d.asset_id = a.asset_id AND d.kind = 'thumb'";

/// 상세 한 건의 본문. `?1` = asset_id.
const DETAIL_SQL: &str = "SELECT a.status, a.hash_state, a.hash_algo, a.content_hash,
        l.rel_path, v.last_mount_point, v.label, f.folder_id
 FROM media_assets a
 LEFT JOIN file_locations l ON l.location_id = (
        SELECT location_id FROM file_locations WHERE asset_id = a.asset_id
        ORDER BY detached_at IS NULL DESC, location_id DESC LIMIT 1)
 LEFT JOIN volumes v ON v.volume_id = l.volume_id
 LEFT JOIN managed_folders f ON f.volume_id = l.volume_id
   AND (f.path_key = '' OR (l.path_key >= f.path_key || '/' AND l.path_key < f.path_key || '0'))
 WHERE a.asset_id = ?1";

/// 같은 해시의 다른 활성 자산 (`idx_media_assets_hash`).
const DUPLICATE_IDS_SQL: &str = "SELECT asset_id FROM media_assets
 WHERE hash_algo = ?1 AND content_hash = ?2 AND asset_id <> ?3 AND status = 'active'
 ORDER BY asset_id LIMIT ?4";

/// 상세에 싣는 중복 자산 최대 수 (D7).
pub const MAX_DUPLICATES: usize = 50;

pub(crate) fn file_name_of(rel_path: &str) -> String {
    rel_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_string()
}

/// 볼륨 연결 끊김이 우선, 그 다음 파일 없음 (D5). `vol` = `volumes.availability`, `presence` = `file_locations.presence`.
pub(crate) fn availability_of(vol: &str, presence: &str) -> Availability {
    if vol == "offline" {
        Availability::Offline
    } else if presence == "missing" {
        Availability::Missing
    } else {
        Availability::Online
    }
}

pub(crate) fn media_kind_of(media_type: &str) -> MediaKind {
    match media_type {
        "video" => MediaKind::Video,
        "audio" => MediaKind::Audio,
        _ => MediaKind::Image,
    }
}

fn summary_from_row(r: &Row<'_>) -> rusqlite::Result<AssetSummary> {
    let media = media_kind_of(&r.get::<_, String>(2)?);
    let availability = availability_of(&r.get::<_, String>(3)?, &r.get::<_, String>(4)?);
    Ok(AssetSummary {
        asset_id: r.get(0)?,
        file_name: file_name_of(&r.get::<_, String>(1)?),
        media_type: media,
        availability,
        width: r.get(5)?,
        height: r.get(6)?,
        duration_ms: r.get(7)?,
        file_size: r.get(8)?,
        captured_at: r.get(9)?,
        modified_at: r.get(10)?,
        favorite: r.get::<_, i64>(11)? == 1,
        thumb_state: r.get(12)?,
        revision: r.get(13)?,
        title: r.get(14)?,
        rating: r.get::<_, i64>(15)?.clamp(0, 5) as u8,
        ai_state: match r.get::<_, i64>(16)? {
            1 => AiCardState::Done,
            2 => AiCardState::Failed,
            _ => AiCardState::None,
        },
    })
}

fn run(
    conn: &Connection,
    sql: &str,
    ids: &[i64],
) -> Result<HashMap<i64, AssetSummary>, IngestError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![json_ids(ids)], summary_from_row)?;
    let mut out = HashMap::with_capacity(ids.len());
    for row in rows {
        let s = row?;
        out.insert(s.asset_id, s);
    }
    Ok(out)
}

/// 검색 결과 페이지 구성. 요청 순서를 유지하며, 활성이 아니거나 활성 위치가 없는 자산(색인 지연)은 뺀다.
pub fn hydrate_page(conn: &Connection, ids: &[i64]) -> Result<Vec<AssetSummary>, IngestError> {
    let mut by_id = run(conn, PAGE_SQL, ids)?;
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

fn related(conn: &Connection, ids: &[i64]) -> Result<Vec<AssetSummary>, IngestError> {
    let mut by_id = run(conn, RELATED_SQL, ids)?;
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

type DetailRow = (
    String,
    String,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// 자산 상세. 없으면 `NotFound`. 삭제·대체된 자산도 조회할 수 있다 (이전/다음 버전 탐색용).
pub fn asset_detail(conn: &Connection, asset_id: i64) -> Result<AssetDetail, IngestError> {
    let summary = related(conn, &[asset_id])?
        .into_iter()
        .next()
        .ok_or(IngestError::NotFound)?;
    let (status, hash_state, hash_algo, hash, rel_path, mount, label, folder_id): DetailRow = conn
        .query_row(DETAIL_SQL, [asset_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })
        .optional()?
        .ok_or(IngestError::NotFound)?;

    let info = media_info(conn, asset_id)?;
    let tags = {
        let mut stmt = conn.prepare_cached(
            "SELECT t.tag_id, t.name FROM asset_tags x JOIN tags t ON t.tag_id = x.tag_id
             WHERE x.asset_id = ?1 ORDER BY t.name_key",
        )?;
        let rows = stmt.query_map([asset_id], |r| {
            Ok(TagRef {
                tag_id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let collections = {
        let mut stmt = conn.prepare_cached(
            "SELECT c.collection_id, c.name FROM collection_assets x
             JOIN collections c ON c.collection_id = x.collection_id
             WHERE x.asset_id = ?1 ORDER BY c.name_key",
        )?;
        let rows = stmt.query_map([asset_id], |r| {
            Ok(CollectionRef {
                collection_id: r.get(0)?,
                name: r.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let memo: String = conn
        .query_row(
            "SELECT memo FROM asset_user_data WHERE asset_id = ?1",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();

    let duplicate_ids = match (hash_algo, hash) {
        (Some(algo), Some(hash)) => {
            let mut stmt = conn.prepare_cached(DUPLICATE_IDS_SQL)?;
            let rows = stmt
                .query_map(params![algo, hash, asset_id, MAX_DUPLICATES as i64], |r| {
                    r.get::<_, i64>(0)
                })?;
            rows.collect::<Result<Vec<_>, _>>()?
        }
        _ => vec![],
    };
    let previous_ids = relation_ids(conn, PREVIOUS_SQL, asset_id)?;
    let newer_ids = relation_ids(conn, NEWER_SQL, asset_id)?;

    let path = match (&mount, &rel_path) {
        (Some(m), Some(rel)) => format!(
            "{}\\{}",
            crate::volume::strip_verbatim(m).trim_end_matches('\\'),
            rel.replace('/', "\\")
        ),
        (None, Some(rel)) => rel.clone(),
        _ => String::new(),
    };
    Ok(AssetDetail {
        summary,
        status,
        path,
        folder_id,
        volume_label: label,
        codec: info.as_ref().and_then(|i| i.codec.clone()),
        audio_codec: info.as_ref().and_then(|i| i.audio_codec.clone()),
        fps: info.as_ref().and_then(|i| i.fps),
        bitrate: info.as_ref().and_then(|i| i.bitrate),
        camera: info.as_ref().and_then(|i| i.camera.clone()),
        extra: info.as_ref().map(|i| i.extra.clone()).unwrap_or_default(),
        playable: info.as_ref().is_some_and(|i| i.playable),
        hash_state,
        tags,
        memo,
        collections,
        duplicates: hydrate_page(conn, &duplicate_ids)?,
        previous_versions: related(conn, &previous_ids)?,
        newer_version: related(conn, &newer_ids)?.into_iter().next(),
    })
}

/// 이 자산이 대체한 구자산 (`idx_asset_relations_to`)
pub const PREVIOUS_SQL: &str = "SELECT from_asset_id FROM asset_relations
 WHERE to_asset_id = ?1 AND relation = 'content_changed' ORDER BY created_at DESC, from_asset_id DESC";

/// 이 자산을 대체한 신자산 (PK 접두)
pub const NEWER_SQL: &str = "SELECT to_asset_id FROM asset_relations
 WHERE from_asset_id = ?1 AND relation = 'content_changed' ORDER BY created_at DESC, to_asset_id DESC LIMIT 1";

fn relation_ids(conn: &Connection, sql: &str, asset_id: i64) -> Result<Vec<i64>, IngestError> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([asset_id], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
