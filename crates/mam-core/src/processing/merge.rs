//! 해시 완료 후 병합 (D6) — P1-4 결함 1 보완.
//!
//! NAS·FAT 처럼 파일 ID 가 없는 볼륨에서 한 번의 스캔 안에 이름이 바뀌면, 새 경로가 신규 자산으로 등록되고
//! 옛 위치는 스캔이 끝난 뒤에야 `missing` 이 된다. 신규 자산의 해시가 계산됐을 때 **같은 해시의 `missing` 자산**이 있고
//! 신규 자산에 **사용자 데이터(태그·메모·즐겨찾기·컬렉션)가 없으면** 신규 자산을 기존 자산에 합친다.
//! 사용자 데이터가 있으면 합치지 않는다 (둘 다 남겨 사용자가 판단하게 한다).
//!
//! 병합 = 기존 자산이 새 경로의 위치를 갖고, 신규 자산은 `deleted`(소프트 삭제)가 된다. 원본 파일은 건드리지 않는다.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::CoreError;
use crate::reconcile::{set_asset_file_state, set_asset_status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeReport {
    /// 위치를 이어받은 기존 자산
    pub kept_asset_id: i64,
    /// 병합돼 `deleted` 가 된 신규 자산 (호출자가 캐시 산출물을 지운다)
    pub removed_asset_id: i64,
}

/// 태그·메모·즐겨찾기·제목·별점·컬렉션 중 하나라도 있는가.
pub fn has_user_data(conn: &Connection, asset_id: i64) -> Result<bool, CoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM asset_tags WHERE asset_id = ?1)
             OR EXISTS (SELECT 1 FROM asset_user_data WHERE asset_id = ?1 AND (memo <> '' OR favorite = 1 OR title IS NOT NULL OR rating > 0))
             OR EXISTS (SELECT 1 FROM collection_assets WHERE asset_id = ?1)",
        [asset_id],
        |r| r.get(0),
    )?)
}

struct NewSide {
    hash: Vec<u8>,
    size: i64,
    modified_at: i64,
    location_id: i64,
    volume_id: i64,
    rel_path: String,
    path_key: String,
    file_identity: Option<String>,
    last_seen_at: i64,
}

/// `new_asset_id` 의 해시가 방금 기록됐다. 병합 조건이 맞으면 병합하고 결과를 돌려준다. 호출자의 트랜잭션 안에서만 부른다.
pub fn merge_after_hash(
    conn: &Connection,
    now: i64,
    new_asset_id: i64,
) -> Result<Option<MergeReport>, CoreError> {
    let new_side: Option<NewSide> = conn
        .query_row(
            "SELECT a.content_hash, a.file_size, a.modified_at,
                    l.location_id, l.volume_id, l.rel_path, l.path_key, l.file_identity, l.last_seen_at
             FROM media_assets a
             JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
             WHERE a.asset_id = ?1 AND a.status = 'active' AND a.hash_algo = 'blake3'
               AND a.content_hash IS NOT NULL AND l.presence = 'present'",
            [new_asset_id],
            |r| {
                Ok(NewSide {
                    hash: r.get(0)?,
                    size: r.get(1)?,
                    modified_at: r.get(2)?,
                    location_id: r.get(3)?,
                    volume_id: r.get(4)?,
                    rel_path: r.get(5)?,
                    path_key: r.get(6)?,
                    file_identity: r.get(7)?,
                    last_seen_at: r.get(8)?,
                })
            },
        )
        .optional()?;
    let Some(new_side) = new_side else {
        return Ok(None);
    };
    // 가장 오래된 후보 하나 - 같은 해시에 missing 이 여럿이면(복사본) 먼저 만든 자산이 원본에 가깝다
    let old: Option<(i64, i64)> = conn
        .query_row(
            "SELECT a.asset_id, l.location_id
             FROM media_assets a
             JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL AND l.presence = 'missing'
             WHERE a.hash_algo = 'blake3' AND a.content_hash = ?1 AND a.status = 'active'
               AND a.file_size = ?2 AND a.asset_id <> ?3
             ORDER BY a.asset_id LIMIT 1",
            params![new_side.hash, new_side.size, new_asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((old_asset_id, old_location_id)) = old else {
        return Ok(None);
    };
    if has_user_data(conn, new_asset_id)? {
        return Ok(None);
    }

    // 유일 인덱스 순서: 기존 자산의 옛 위치와 신규 자산의 위치를 먼저 detach 한 뒤 기존 자산에 새 위치를 만든다
    for location_id in [old_location_id, new_side.location_id] {
        conn.execute(
            "UPDATE file_locations SET detached_at = ?2 WHERE location_id = ?1",
            params![location_id, now],
        )?;
    }
    conn.execute(
        "INSERT INTO file_locations
           (asset_id, volume_id, rel_path, path_key, file_identity, presence, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'present', ?6)",
        params![
            old_asset_id,
            new_side.volume_id,
            new_side.rel_path,
            new_side.path_key,
            new_side.file_identity,
            new_side.last_seen_at
        ],
    )?;
    set_asset_file_state(conn, now, old_asset_id, new_side.size, new_side.modified_at)?;
    set_asset_status(conn, now, new_asset_id, "deleted")?;
    // 신규 자산의 처리 상태·대기 작업은 의미가 없어졌다 (실행 중인 작업은 비활성 자산이라 스스로 끝난다)
    conn.execute(
        "DELETE FROM asset_derivatives WHERE asset_id = ?1",
        [new_asset_id],
    )?;
    conn.execute(
        "DELETE FROM jobs WHERE asset_id = ?1 AND status = 'pending' AND kind IN ('meta', 'thumb', 'hash')",
        [new_asset_id],
    )?;
    Ok(Some(MergeReport {
        kept_asset_id: old_asset_id,
        removed_asset_id: new_asset_id,
    }))
}
