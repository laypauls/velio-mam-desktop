//! 별점 0~5 (`asset_user_data.rating`). 0 = 미평가 단일 상태 — 행이 없어도 0 이다.

use rusqlite::{Connection, params};

use super::{UserData, bump_assets, clean_ids, json_ids, require_assets, returning_ids};
use crate::scan::{IngestError, TOTAL_COUNT_SQL, not_in_hidden_folder};

/// 별점 최댓값 (포함)
pub const MAX_RATING: u8 = 5;

/// 갤러리 기본 필터(오프라인 포함, 파일 없음 제외)와 같은 기준의 후보: 활성 자산 + 활성 위치(present) + 숨김 폴더 밖.
/// `uq_file_locations_active_asset` 때문에 자산당 최대 1행이라 중복 집계가 없다.
const RATED_COUNTS_SQL: &str = concat!(
    "SELECT u.rating, count(*)
 FROM asset_user_data u
 JOIN media_assets a ON a.asset_id = u.asset_id AND a.status = 'active'
 JOIN file_locations l ON l.asset_id = u.asset_id AND l.detached_at IS NULL AND l.presence = 'present'
 WHERE u.rating > 0 AND ",
    not_in_hidden_folder!(),
    "
 GROUP BY u.rating"
);

impl UserData {
    /// 선택한 자산의 별점을 한 트랜잭션으로 설정한다 (0 = 지우기, 행은 지우지 않는다).
    /// 값이 실제로 바뀐 자산만 revision 증가 + outbox (고우선순위). 같은 값 재설정은 no-op.
    pub fn set_rating(&self, asset_ids: &[i64], rating: u8) -> Result<(), IngestError> {
        if rating > MAX_RATING {
            return Err(IngestError::InvalidInput("rating"));
        }
        let ids = clean_ids(asset_ids)?;
        if ids.is_empty() {
            return Ok(());
        }
        self.write(move |tx, now| {
            require_assets(tx, &ids)?;
            let json = json_ids(&ids);
            let changed = if rating > 0 {
                returning_ids(
                    tx,
                    "INSERT INTO asset_user_data (asset_id, rating, updated_at)
                     SELECT value, ?3, ?2 FROM json_each(?1) WHERE true
                     ON CONFLICT (asset_id) DO UPDATE
                       SET rating = excluded.rating, updated_at = excluded.updated_at
                       WHERE rating <> excluded.rating
                     RETURNING asset_id",
                    params![json, now, rating],
                )?
            } else {
                returning_ids(
                    tx,
                    "UPDATE asset_user_data SET rating = 0, updated_at = ?2
                     WHERE rating <> 0 AND asset_id IN (SELECT value FROM json_each(?1))
                     RETURNING asset_id",
                    params![json, now],
                )?
            };
            bump_assets(tx, now, &changed, 0)
        })
    }

    /// 사이드바 개수: 길이 6, 인덱스 = 별점 (0 = 미평가). 활성 자산 중 파일 없음 제외, 오프라인 포함.
    /// 행이 없는 자산은 미평가로 센다 (= 전체 - 평가됨).
    pub fn rating_counts(&self) -> Result<[u64; 6], IngestError> {
        self.read(counts)
    }
}

fn counts(c: &Connection) -> Result<[u64; 6], IngestError> {
    let mut out = [0u64; 6];
    let mut stmt = c.prepare_cached(RATED_COUNTS_SQL)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut rated = 0u64;
    for row in rows {
        let (rating, n) = row?;
        let n = u64::try_from(n).unwrap_or(0);
        if let Some(slot) = usize::try_from(rating).ok().and_then(|i| out.get_mut(i)) {
            *slot = n;
            rated += n;
        }
    }
    let total: i64 = c.query_row(TOTAL_COUNT_SQL, [], |r| r.get(0))?;
    out[0] = u64::try_from(total).unwrap_or(0).saturating_sub(rated);
    Ok(out)
}

/// EXPLAIN QUERY PLAN 검증용 (테스트)
#[doc(hidden)]
pub const COUNT_SQLS: [&str; 2] = [RATED_COUNTS_SQL, TOTAL_COUNT_SQL];
