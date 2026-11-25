//! reconcile 이 쓰는 SQL. 자산을 바꾸는 함수는 `revision` 증가와 `search_outbox` 기록을 함께 수행하므로
//! 반드시 호출자의 트랜잭션 안에서 실행한다 (db.md §3).

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::CoreError;

/// 스캔에서 발생한 outbox 이벤트의 우선순위 (사용자 편집 0 보다 낮다)
pub(super) const SCAN_PRIORITY: i64 = 1;

#[derive(Debug, Clone)]
pub(super) struct AssetRow {
    pub hash: Option<[u8; 32]>,
    pub size: i64,
    pub modified_at: i64,
}

#[derive(Debug, Clone)]
pub(super) struct LocRow {
    pub location_id: i64,
    pub asset_id: i64,
    pub rel_path: String,
    pub file_identity: Option<String>,
    pub presence: String,
    pub last_seen_at: i64,
}

fn loc_from(r: &Row<'_>) -> rusqlite::Result<LocRow> {
    Ok(LocRow {
        location_id: r.get(0)?,
        asset_id: r.get(1)?,
        rel_path: r.get(2)?,
        file_identity: r.get(3)?,
        presence: r.get(4)?,
        last_seen_at: r.get(5)?,
    })
}

const LOC_COLS: &str = "location_id, asset_id, rel_path, file_identity, presence, last_seen_at";

pub(super) fn asset(conn: &Connection, asset_id: i64) -> Result<AssetRow, CoreError> {
    Ok(conn.query_row(
        "SELECT content_hash, file_size, modified_at FROM media_assets WHERE asset_id = ?1",
        [asset_id],
        |r| {
            let blob: Option<Vec<u8>> = r.get(0)?;
            Ok(AssetRow {
                hash: blob.and_then(|b| <[u8; 32]>::try_from(b).ok()),
                size: r.get(1)?,
                modified_at: r.get(2)?,
            })
        },
    )?)
}

pub(super) fn active_location_at(
    conn: &Connection,
    volume_id: i64,
    path_key: &str,
) -> Result<Option<LocRow>, CoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {LOC_COLS} FROM file_locations
                 WHERE volume_id = ?1 AND path_key = ?2 AND detached_at IS NULL"
            ),
            params![volume_id, path_key],
            loc_from,
        )
        .optional()?)
}

/// 파일 ID 로 활성 위치 찾기. `INDEXED BY` 로 파일 ID 부분 인덱스를 강제한다 — 없으면 플래너가 `volume_id` 인덱스를 골라
/// 새 파일마다 볼륨 전체를 훑는다(1만 건 최초 스캔 44초, P2-2 측정). 계획은 아래 시험이 고정한다.
const BY_IDENTITY_SQL: &str = "FROM file_locations INDEXED BY idx_file_locations_identity
     WHERE file_identity = ?1 AND volume_id = ?2 AND detached_at IS NULL ORDER BY location_id";

pub(super) fn active_locations_by_identity(
    conn: &Connection,
    volume_id: i64,
    identity: &str,
) -> Result<Vec<LocRow>, CoreError> {
    let mut stmt = conn.prepare(&format!("SELECT {LOC_COLS} {BY_IDENTITY_SQL}"))?;
    let rows = stmt.query_map(params![identity, volume_id], loc_from)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// 같은 볼륨·경로에 detach 된 위치가 있고, 그 자산이 활성이며 지금 활성 위치가 없고 크기·수정 시각이 같으면 그 자산 (D9).
/// 폴더 제거(위치 detach) 뒤 같은 폴더를 다시 등록했을 때 태그·메모가 끊기지 않게 한다.
/// `INDEXED BY` 로 detach 위치 부분 인덱스를 강제한다 (EXPLAIN 시험이 고정).
pub(super) const DETACHED_AT_SQL: &str = "FROM file_locations l INDEXED BY idx_file_locations_detached_path
     JOIN media_assets a ON a.asset_id = l.asset_id
     WHERE l.volume_id = ?1 AND l.path_key = ?2 AND l.detached_at IS NOT NULL
       AND a.status = 'active' AND a.file_size = ?3 AND a.modified_at = ?4
       AND NOT EXISTS (SELECT 1 FROM file_locations x WHERE x.asset_id = a.asset_id AND x.detached_at IS NULL)
     ORDER BY l.location_id DESC LIMIT 1";

pub(super) fn detached_asset_at(
    conn: &Connection,
    volume_id: i64,
    path_key: &str,
    size: i64,
    modified_at: i64,
) -> Result<Option<i64>, CoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT l.asset_id {DETACHED_AT_SQL}"),
            params![volume_id, path_key, size, modified_at],
            |r| r.get(0),
        )
        .optional()?)
}

/// 활성(`active`) 자산인지
pub(super) fn is_active_asset(conn: &Connection, asset_id: i64) -> Result<bool, CoreError> {
    Ok(conn.query_row(
        "SELECT status = 'active' FROM media_assets WHERE asset_id = ?1",
        [asset_id],
        |r| r.get(0),
    )?)
}

/// 위치가 아직 활성이며 `missing` 인지 (후보 재검증)
pub(super) fn is_active_missing_location(
    conn: &Connection,
    location_id: i64,
) -> Result<bool, CoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM file_locations
             WHERE location_id = ?1 AND detached_at IS NULL AND presence = 'missing'",
            [location_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn emit_outbox(conn: &Connection, now: i64, asset_id: i64, revision: i64) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, 'upsert', ?3, ?4)",
        params![asset_id, revision, SCAN_PRIORITY, now],
    )?;
    Ok(())
}

/// 새 자산 (revision 1) + outbox. `hash` 가 없으면 `pending`.
pub(super) fn insert_asset(
    conn: &Connection,
    now: i64,
    media_type: &str,
    size: i64,
    modified_at: i64,
    hash: Option<&[u8; 32]>,
    edge_sig: Option<&[u8; 32]>,
) -> Result<i64, CoreError> {
    let (algo, state) = if hash.is_some() {
        (Some("blake3"), "done")
    } else {
        (None, "pending")
    };
    conn.execute(
        "INSERT INTO media_assets
           (media_type, content_hash, hash_algo, hash_state, file_size, modified_at, created_at, updated_at, edge_sig)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        params![
            media_type,
            hash.map(|h| h.as_slice()),
            algo,
            state,
            size,
            modified_at,
            now,
            edge_sig.map(|e| e.as_slice())
        ],
    )?;
    let asset_id = conn.last_insert_rowid();
    emit_outbox(conn, now, asset_id, 1)?;
    Ok(asset_id)
}

/// 자산 변경 내용 (None = 유지)
#[derive(Default)]
pub(super) struct AssetChange<'a> {
    pub size: Option<i64>,
    pub modified_at: Option<i64>,
    pub status: Option<&'a str>,
    pub hash: Option<&'a [u8; 32]>,
}

/// 자산 갱신 + `revision` 증가 + outbox (한 호출 = 한 이벤트). 변경 내용이 없어도 revision 을 올린다
/// (위치 상태 변화처럼 자산 컬럼 밖의 변경을 색인에 알릴 때 쓴다).
pub(super) fn update_asset(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    change: AssetChange<'_>,
) -> Result<i64, CoreError> {
    let revision: i64 = conn.query_row(
        "UPDATE media_assets SET
           file_size    = COALESCE(?2, file_size),
           modified_at  = COALESCE(?3, modified_at),
           status       = COALESCE(?4, status),
           content_hash = COALESCE(?5, content_hash),
           hash_algo    = CASE WHEN ?5 IS NULL THEN hash_algo ELSE 'blake3' END,
           hash_state   = CASE WHEN ?5 IS NULL THEN hash_state ELSE 'done' END,
           revision     = revision + 1,
           updated_at   = ?6
         WHERE asset_id = ?1
         RETURNING revision",
        params![
            asset_id,
            change.size,
            change.modified_at,
            change.status,
            change.hash.map(|h| h.as_slice()),
            now
        ],
        |r| r.get(0),
    )?;
    emit_outbox(conn, now, asset_id, revision)?;
    Ok(revision)
}

pub(super) struct NewLocation<'a> {
    pub asset_id: i64,
    pub volume_id: i64,
    pub rel_path: &'a str,
    pub path_key: &'a str,
    pub file_identity: Option<&'a str>,
    pub seen_at: i64,
}

pub(super) fn insert_location(conn: &Connection, loc: &NewLocation<'_>) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO file_locations
           (asset_id, volume_id, rel_path, path_key, file_identity, presence, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'present', ?6)",
        params![
            loc.asset_id,
            loc.volume_id,
            loc.rel_path,
            loc.path_key,
            loc.file_identity,
            loc.seen_at
        ],
    )?;
    Ok(())
}

pub(super) fn detach_location(
    conn: &Connection,
    now: i64,
    location_id: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE file_locations SET detached_at = ?2 WHERE location_id = ?1",
        params![location_id, now],
    )?;
    Ok(())
}

/// 스캔에서 다시 본 위치: 표기·식별자를 갱신하고 `present` 로 되돌린다.
pub(super) fn mark_location_seen(
    conn: &Connection,
    location_id: i64,
    rel_path: &str,
    file_identity: Option<&str>,
    seen_at: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE file_locations SET
           rel_path = ?2, file_identity = COALESCE(?3, file_identity),
           presence = 'present', last_seen_at = ?4
         WHERE location_id = ?1",
        params![location_id, rel_path, file_identity, seen_at],
    )?;
    Ok(())
}

/// 해시 보류(`HashDeferred`) 파일도 "이번 스캔에서 봤다" 로 기록해 `missing` 오판을 막는다.
pub(super) fn mark_seen_at_path(
    conn: &Connection,
    volume_id: i64,
    path_key: &str,
    seen_at: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "UPDATE file_locations SET last_seen_at = ?3
         WHERE volume_id = ?1 AND path_key = ?2 AND detached_at IS NULL AND last_seen_at < ?3",
        params![volume_id, path_key, seen_at],
    )?;
    Ok(())
}

pub(super) fn insert_relation(
    conn: &Connection,
    now: i64,
    from_asset_id: i64,
    to_asset_id: i64,
) -> Result<(), CoreError> {
    conn.execute(
        "INSERT INTO asset_relations (from_asset_id, to_asset_id, relation, created_at)
         VALUES (?1, ?2, 'content_changed', ?3)",
        params![from_asset_id, to_asset_id, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! 분류: Performance 계약 (회귀) — 파일 ID 조회가 볼륨 전체를 훑지 않는다
    use super::*;

    #[test]
    fn t_scan_perf_01_identity_lookup_uses_the_identity_index_not_the_volume_index() {
        // Arrange: 실제 마이그레이션이 적용된 DB
        let dir = std::env::temp_dir().join(format!("mam-core-plan-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut conn = mam_db::open(&dir.join("t.sqlite")).unwrap();
        mam_db::migrate(&mut conn, &dir.join("backups"), &mam_db::SystemClock).unwrap();

        // Act
        let mut stmt = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT {LOC_COLS} {BY_IDENTITY_SQL}"
            ))
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params!["a:b", 1], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(stmt);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);

        // Assert
        let joined = plan.join(" | ");
        assert!(joined.contains("idx_file_locations_identity"), "{joined}");
        assert!(!joined.contains("idx_file_locations_volume"), "{joined}");
    }

    #[test]
    fn t_scan_perf_02_detached_lookup_uses_the_detached_path_index_not_a_volume_scan() {
        // Arrange: 실제 마이그레이션이 적용된 DB
        let dir = std::env::temp_dir().join(format!("mam-core-plan2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut conn = mam_db::open(&dir.join("t.sqlite")).unwrap();
        mam_db::migrate(&mut conn, &dir.join("backups"), &mam_db::SystemClock).unwrap();

        // Act: 실제 코드가 쓰는 SQL 상수의 계획
        let mut stmt = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT l.asset_id {DETACHED_AT_SQL}"
            ))
            .unwrap();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params![1, "a/b.jpg", 10, 5], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        drop(stmt);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);

        // Assert: 새 경로마다 볼륨 전체를 훑으면 최초 스캔이 O(n^2) 가 된다 (P2-2 결함 1 과 같은 패턴)
        let joined = plan.join(" | ");
        assert!(
            joined.contains("idx_file_locations_detached_path"),
            "{joined}"
        );
        assert!(!joined.contains("idx_file_locations_volume"), "{joined}");
    }
}
