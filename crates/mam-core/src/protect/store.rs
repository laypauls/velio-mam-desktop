//! 보호 작업의 DB 쓰기. 자산 변경 + `revision` 증가 + `search_outbox` 는 호출자의 한 트랜잭션 안에서 실행한다 (db.md §3).

use rusqlite::{Connection, OptionalExtension, params};

/// 사용자 명령에서 나온 outbox 이벤트는 고우선순위 (db.md §3)
const USER_PRIORITY: i64 = 0;

fn bump_revision(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    status: Option<&str>,
    size_mtime: Option<(i64, i64)>,
) -> rusqlite::Result<i64> {
    conn.query_row(
        "UPDATE media_assets SET
           status      = COALESCE(?2, status),
           file_size   = COALESCE(?3, file_size),
           modified_at = COALESCE(?4, modified_at),
           revision    = revision + 1,
           updated_at  = ?5
         WHERE asset_id = ?1
         RETURNING revision",
        params![
            asset_id,
            status,
            size_mtime.map(|s| s.0),
            size_mtime.map(|s| s.1),
            now
        ],
        |r| r.get(0),
    )
}

fn emit(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    revision: i64,
    event: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![asset_id, revision, event, USER_PRIORITY, now],
    )?;
    Ok(())
}

fn detach(conn: &Connection, now: i64, location_id: i64) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE file_locations SET detached_at = ?2 WHERE location_id = ?1",
        params![location_id, now],
    )?;
    Ok(())
}

/// 휴지통으로 보낸 뒤: 위치 detach + `status = deleted` + revision + `delete` 이벤트
pub(super) fn mark_deleted(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    location_id: i64,
) -> rusqlite::Result<()> {
    detach(conn, now, location_id)?;
    let revision = bump_revision(conn, now, asset_id, Some("deleted"), None)?;
    emit(conn, now, asset_id, revision, "delete")
}

/// 새 위치 (관리 폴더 안으로 이동한 경우)
pub(super) struct NewPlace<'a> {
    pub volume_id: i64,
    pub rel_path: &'a str,
    pub path_key: &'a str,
    pub file_identity: Option<&'a str>,
    pub size: i64,
    pub modified_at: i64,
}

/// 같은 위치 행을 새 경로로 옮겨 자산 연결을 유지한다 (+ revision, `upsert` 이벤트)
pub(super) fn relink(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    location_id: i64,
    to: &NewPlace<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE file_locations SET
           volume_id = ?2, rel_path = ?3, path_key = ?4, file_identity = ?5,
           presence = 'present', last_seen_at = ?6
         WHERE location_id = ?1",
        params![
            location_id,
            to.volume_id,
            to.rel_path,
            to.path_key,
            to.file_identity,
            now
        ],
    )?;
    let revision = bump_revision(conn, now, asset_id, None, Some((to.size, to.modified_at)))?;
    emit(conn, now, asset_id, revision, "upsert")
}

/// 관리 폴더 밖으로 이동: 위치만 detach, 자산(태그·메모)은 유지 (+ revision, `upsert`)
pub(super) fn detach_moved_out(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    location_id: i64,
) -> rusqlite::Result<()> {
    detach(conn, now, location_id)?;
    let revision = bump_revision(conn, now, asset_id, None, None)?;
    emit(conn, now, asset_id, revision, "upsert")
}

/// 이 경로에 이미 활성 위치가 있는가 (이동 대상 충돌 사전 점검)
pub(super) fn active_at(
    conn: &Connection,
    volume_id: i64,
    path_key: &str,
) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM file_locations
             WHERE volume_id = ?1 AND path_key = ?2 AND detached_at IS NULL",
            params![volume_id, path_key],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// 볼륨 키 -> `volume_id`
pub(super) fn volume_id_by_key(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT volume_id FROM volumes WHERE volume_key = ?1",
        [key],
        |r| r.get(0),
    )
    .optional()
}

/// 이 경로가 사용 중인 관리 폴더 안인가 (`dir_key` = 볼륨 기준 디렉터리 path_key, 볼륨 루트는 '')
pub(super) fn is_managed(
    conn: &Connection,
    volume_id: i64,
    dir_key: &str,
) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT path_key FROM managed_folders WHERE volume_id = ?1 AND enabled = 1",
    )?;
    let roots = stmt.query_map([volume_id], |r| r.get::<_, String>(0))?;
    for root in roots {
        let root = root?;
        if root.is_empty()
            || dir_key == root
            || dir_key
                .strip_prefix(&root)
                .is_some_and(|rest| rest.starts_with('/'))
        {
            return Ok(true);
        }
    }
    Ok(false)
}
