//! 앞뒤 4K 체크섬(`media_assets.edge_sig`) 저장·조회 — 전체 해시 대상을 좁히는 후보 축소 단계.
//!
//! 같은 크기·같은 `edge_sig` 의 활성 자산이 있을 때만 전체 해시가 필요하다. 서로 다르면 내용이 다르다.
//! 자산 컬럼을 바꾸지만 검색 색인에 영향이 없으므로 `revision` 은 올리지 않는다 (outbox 폭증 방지).

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::CoreError;
use crate::jobs::JobKind;

/// 저장된 edge_sig (없거나 NULL 이면 `None`).
pub(super) fn stored_edge(conn: &Connection, asset_id: i64) -> Result<Option<[u8; 32]>, CoreError> {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT edge_sig FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(blob.and_then(|b| <[u8; 32]>::try_from(b).ok()))
}

/// 같은 크기·같은 edge_sig 의 다른 활성 자산이 있는가 (`idx_media_assets_edge`).
pub(super) const COLLISION_SQL: &str = "SELECT EXISTS (
       SELECT 1 FROM media_assets
       WHERE status = 'active' AND file_size = ?1 AND edge_sig = ?2 AND asset_id <> ?3)";

/// edge_sig 를 방금 저장했다: 같은 크기·같은 sig 이면서 전체 해시가 없는 상대에게 `hash` 작업을 등록한다
/// (상대 파일을 다시 읽지 않는다 — 상대 edge_sig 는 이미 DB 에 있다). 이미 대기 작업이 있으면 무시된다.
pub(super) const ENQUEUE_COUNTERPARTS_SQL: &str =
    "INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at)
     SELECT 'hash', asset_id, ?1, ?2, ?2 FROM media_assets
     WHERE status = 'active' AND hash_state = 'pending'
       AND file_size = ?3 AND edge_sig = ?4 AND asset_id <> ?5";

/// 결과: `None` = 자산이 그 사이 바뀌었다 (크기·수정 시각·상태 불일치) — 아무것도 하지 않는다.
/// `Some(true)` = 같은 크기·같은 edge_sig 상대가 있다 (전체 해시 필요).
///
/// `new_sig` 가 있으면 저장하고(NULL 일 때만) 상대 작업을 등록한다. 없으면 저장된 값으로 충돌만 확인한다.
pub(super) fn record_and_check(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    size: i64,
    modified_at: i64,
    new_sig: Option<[u8; 32]>,
) -> Result<Option<bool>, CoreError> {
    let sig = match new_sig {
        Some(sig) => {
            let n = conn.execute(
                "UPDATE media_assets SET edge_sig = ?2
                 WHERE asset_id = ?1 AND status = 'active' AND file_size = ?3 AND modified_at = ?4
                   AND edge_sig IS NULL",
                params![asset_id, sig.as_slice(), size, modified_at],
            )?;
            if n == 0 {
                return Ok(None);
            }
            conn.execute(
                ENQUEUE_COUNTERPARTS_SQL,
                params![
                    JobKind::Hash.default_priority(),
                    now,
                    size,
                    sig.as_slice(),
                    asset_id
                ],
            )?;
            sig
        }
        None => match stored_edge(conn, asset_id)? {
            Some(sig) => sig,
            None => return Ok(None),
        },
    };
    Ok(Some(conn.query_row(
        COLLISION_SQL,
        params![size, sig.as_slice(), asset_id],
        |r| r.get(0),
    )?))
}
