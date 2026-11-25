//! 컬렉션 조회 — 다중 자산 소속 집계(`collection_membership`)와 연속 재생 대기열(`collection_play_queue`).
//! 둘 다 읽기 전용이다 (순서 정규화 같은 쓰기를 하지 않는다).

use rusqlite::params;
use serde::Serialize;

use super::collections::require_collection;
use super::{UserData, clean_ids, json_ids};
use crate::query::{Availability, MediaKind, availability_of, file_name_of, media_kind_of};
use crate::scan::IngestError;

/// `collection_membership` 항목 (계약 `CollectionMembership`) — 선택 자산 중 이 컬렉션에 속한 수. 0 인 컬렉션은 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionMembership {
    pub collection_id: i64,
    pub count: i64,
}

/// `collection_play_queue` 항목 (계약 `PlayQueueItem`) — 목록 표시에 필요한 가벼운 필드만.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayQueueItem {
    pub asset_id: i64,
    pub media_type: MediaKind,
    pub availability: Availability,
    pub file_name: String,
    pub title: Option<String>,
    pub duration_ms: Option<i64>,
}

/// 구동 테이블을 `json_each` 로 고정하고 `idx_collection_assets_asset` 커버링 인덱스를 쓴다.
pub const MEMBERSHIP_SQL: &str = "SELECT x.collection_id, count(*) FROM json_each(?1) j
 CROSS JOIN collection_assets x ON x.asset_id = j.value
 GROUP BY x.collection_id ORDER BY x.collection_id";

/// 활성 자산·활성 위치만 (오프라인·파일 없음 포함 — 재생 쪽이 건너뜀 표시). 순서는 표준 순서.
pub const PLAY_QUEUE_SQL: &str = "SELECT x.asset_id, a.media_type, v.availability, l.presence, l.rel_path, u.title, m.duration_ms
 FROM collection_assets x
 JOIN media_assets a ON a.asset_id = x.asset_id AND a.status = 'active'
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 JOIN volumes v ON v.volume_id = l.volume_id
 LEFT JOIN asset_user_data u ON u.asset_id = a.asset_id
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
 WHERE x.collection_id = ?1
 ORDER BY x.position IS NULL, x.position, x.added_at, x.asset_id";

/// 재생 대기열 조회 동안만 쓰는 읽기 설정: 메모리 매핑 2 GiB + 페이지 캐시 256 MiB.
/// 조인이 10만 행을 무작위 조회해 기본 설정(캐시 2 MB, 매핑 없음)으로는 1M 자산에서 ~1.8 s 걸린다.
/// 측정: 캐시만 올리면 호출마다 캐시를 새로 채워 P95 ~680 ms, mmap 만 P95 ~510 ms, 둘 다 P95 ~450 ms.
const PLAY_QUEUE_MMAP_BYTES: i64 = 2 << 30;
const PLAY_QUEUE_CACHE_KIB: i64 = 256 * 1024;

/// 읽기 연결의 `mmap_size`·`cache_size` 를 올렸다가 Drop 에서 원래 값으로 되돌린다 (오류·패닉 경로 포함).
/// 풀은 연결을 호출자에게 독점으로 빌려주므로(`DbPool::read`) 겹치는 호출도 서로의 연결을 건드리지 않고,
/// 되돌린 뒤에 풀로 반환돼 다른 쿼리로 새지 않는다.
struct ReadBoost<'a> {
    conn: &'a rusqlite::Connection,
    mmap: i64,
    cache: i64,
}

impl<'a> ReadBoost<'a> {
    fn raise(conn: &'a rusqlite::Connection) -> Result<Self, IngestError> {
        let mmap: i64 = conn.query_row("PRAGMA mmap_size", [], |r| r.get(0))?;
        let cache: i64 = conn.query_row("PRAGMA cache_size", [], |r| r.get(0))?;
        // 가드를 먼저 만들어 아래 설정이 중간에 실패해도 Drop 이 되돌린다
        let guard = Self { conn, mmap, cache };
        conn.pragma_update(None, "cache_size", -PLAY_QUEUE_CACHE_KIB)?;
        set_mmap(conn, PLAY_QUEUE_MMAP_BYTES)?;
        Ok(guard)
    }
}

fn set_mmap(conn: &rusqlite::Connection, bytes: i64) -> rusqlite::Result<()> {
    // `PRAGMA mmap_size = N` 은 결과 행을 돌려준다
    conn.query_row(&format!("PRAGMA mmap_size = {bytes}"), [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|_| ())
}

impl Drop for ReadBoost<'_> {
    fn drop(&mut self) {
        // 되돌리기 실패는 성능에만 영향 (정확성 무관) — 무시한다
        let _ = set_mmap(self.conn, self.mmap);
        let _ = self.conn.pragma_update(None, "cache_size", self.cache);
    }
}

impl UserData {
    /// 선택한 자산들의 컬렉션별 소속 수. 중복 id 는 한 번만 센다. 빈 입력 → 빈 목록.
    pub fn collection_membership(
        &self,
        asset_ids: &[i64],
    ) -> Result<Vec<CollectionMembership>, IngestError> {
        let ids = clean_ids(asset_ids)?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read(|c| {
            let mut stmt = c.prepare_cached(MEMBERSHIP_SQL)?;
            let rows = stmt.query_map([json_ids(&ids)], |r| {
                Ok(CollectionMembership {
                    collection_id: r.get(0)?,
                    count: r.get(1)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// 컬렉션 순서 그대로의 재생 대기열. 없는 컬렉션은 `NotFound`.
    pub fn collection_play_queue(&self, id: i64) -> Result<Vec<PlayQueueItem>, IngestError> {
        self.read(|c| {
            let _boost = ReadBoost::raise(c)?;
            require_collection(c, id)?;
            let mut stmt = c.prepare_cached(PLAY_QUEUE_SQL)?;
            let rows = stmt.query_map(params![id], |r| {
                Ok(PlayQueueItem {
                    asset_id: r.get(0)?,
                    media_type: media_kind_of(&r.get::<_, String>(1)?),
                    availability: availability_of(&r.get::<_, String>(2)?, &r.get::<_, String>(3)?),
                    file_name: file_name_of(&r.get::<_, String>(4)?),
                    title: r.get(5)?,
                    duration_ms: r.get(6)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }
}
